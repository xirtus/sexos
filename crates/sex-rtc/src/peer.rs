// SPDX-License-Identifier: GPL-3.0-only
//! One RTCPeerConnection: transceivers, JSEP offer/answer, one bundled ICE agent with
//! DTLS-SRTP on top, RTP and RTCP for each transceiver, and the bandwidth estimate.
//!
//! No I/O. The browser binds a UDP socket when `wants_transport` says so, hands the
//! addresses in with `start_transport`, feeds datagrams to `receive`, sends what
//! `poll_transmit` gives, and calls `handle_timeout` at `poll_timeout`. Pictures and
//! sound go in with `send_frame`; received frames come out of `poll_frame`.
use crate::{Codec, Kind};
use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt::Write;
use core::net::{IpAddr, SocketAddr};
use rand_chacha::ChaCha20Rng;
use rand_core::{RngCore, SeedableRng};
use sex_dtls::{Certificate, Dtls, Fingerprint, Role};
use sex_ice::{Agent, Route};
use sex_rtp::codec::{Depacketizer, Packetizer};
use sex_rtp::rtcp::ReceiveStats;
use sex_rtp::{BandwidthEstimator, JitterBuffer, JitterConfig, Packet, Rtcp, RtpStream, TwccRecorder};
use sex_sdp::{Attributes, Direction, Session};

pub const ABS_SEND_TIME: &str = "http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time";
pub const TWCC: &str = "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01";
pub const SDES_MID: &str = "urn:ietf:params:rtp-hdrext:sdes:mid";
const MTU: usize = 1200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signaling { Stable, HaveLocalOffer, HaveRemoteOffer, HaveLocalPranswer, HaveRemotePranswer, Closed }
impl Signaling {
    pub fn as_str(self) -> &'static str {
        match self {
            Signaling::Stable => "stable", Signaling::HaveLocalOffer => "have-local-offer", Signaling::HaveRemoteOffer => "have-remote-offer",
            Signaling::HaveLocalPranswer => "have-local-pranswer", Signaling::HaveRemotePranswer => "have-remote-pranswer", Signaling::Closed => "closed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SdpType { Offer, Answer, Pranswer, Rollback }
impl SdpType {
    pub fn parse(s: &str) -> Option<SdpType> {
        Some(match s { "offer" => SdpType::Offer, "answer" => SdpType::Answer, "pranswer" => SdpType::Pranswer, "rollback" => SdpType::Rollback, _ => return None })
    }
    pub fn as_str(self) -> &'static str {
        match self { SdpType::Offer => "offer", SdpType::Answer => "answer", SdpType::Pranswer => "pranswer", SdpType::Rollback => "rollback" }
    }
}

/// An error as the page sees it: the DOMException name and a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error { pub name: &'static str, pub message: String }
fn err(name: &'static str, message: &str) -> Error { Error { name, message: message.into() } }

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerKind { Stun, Turn(sex_ice::turn::Transport) }

/// One ICE server address from `iceServers`, to be resolved by the browser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Server { pub kind: ServerKind, pub host: String, pub port: u16, pub username: String, pub credential: String }

impl Server {
    /// `stun:host[:port]`, `turn:host[:port][?transport=tcp]`, `turns:host[:port]`.
    pub fn parse(url: &str, username: &str, credential: &str) -> Option<Server> {
        let (scheme, rest) = url.split_once(':')?;
        let (hostport, query) = rest.split_once('?').unwrap_or((rest, ""));
        let tcp = query.split('&').any(|q| q.eq_ignore_ascii_case("transport=tcp"));
        let (kind, default) = match scheme.to_ascii_lowercase().as_str() {
            "stun" => (ServerKind::Stun, 3478),
            "stuns" => return None,
            "turn" => (ServerKind::Turn(if tcp { sex_ice::turn::Transport::Tcp } else { sex_ice::turn::Transport::Udp }), 3478),
            "turns" => (ServerKind::Turn(sex_ice::turn::Transport::Tls), 5349),
            _ => return None,
        };
        let (host, port) = if let Some(h) = hostport.strip_prefix('[') {
            let (h, p) = h.split_once(']')?;
            (h, p.strip_prefix(':').and_then(|p| p.parse().ok()).unwrap_or(default))
        } else {
            match hostport.rsplit_once(':') { Some((h, p)) => (h, p.parse().ok()?), None => (hostport, default) }
        };
        if host.is_empty() { return None; }
        Some(Server { kind, host: host.into(), port, username: username.into(), credential: credential.into() })
    }
}

#[derive(Clone, Debug, Default)]
pub struct Config {
    pub servers: Vec<Server>,
    /// iceTransportPolicy "relay".
    pub relay_only: bool,
}

/// One `sendEncodings` entry. Simulcast rids are kept, but one layer is sent.
#[derive(Clone, Debug, PartialEq)]
pub struct Encoding { pub rid: String, pub active: bool, pub max_bitrate: Option<u32>, pub scale_down: f32, pub max_framerate: Option<u32> }
impl Default for Encoding {
    fn default() -> Self { Encoding { rid: String::new(), active: true, max_bitrate: None, scale_down: 1.0, max_framerate: None } }
}

/// A codec both sides agreed on, with its payload type and feedback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Negotiated { pub codec: Codec, pub pt: u8, pub fmtp: Option<String>, pub nack: bool, pub pli: bool, pub fir: bool, pub remb: bool, pub twcc: bool }

#[derive(Clone, Debug, Default)]
pub struct SendStats {
    pub bytes: u64, pub packets: u64, pub header_bytes: u64, pub frames: u64, pub keyframes: u64,
    pub nacks: u64, pub plis: u64, pub firs: u64, pub retransmitted_bytes: u64, pub retransmitted_packets: u64,
    pub width: u32, pub height: u32,
}
#[derive(Clone, Debug, Default)]
pub struct RecvStats {
    pub bytes: u64, pub packets: u64, pub header_bytes: u64, pub frames: u64, pub keyframes: u64, pub decoded: u64,
    pub nacks: u64, pub plis: u64, pub firs: u64, pub width: u32, pub height: u32, pub last_packet_ms: u64,
}

enum Depack { Opus(sex_rtp::codec::Opus), Vp8(sex_rtp::codec::Vp8), Vp9(sex_rtp::codec::Vp9), H264(sex_rtp::codec::H264) }
impl Depack {
    fn new(c: Codec) -> Depack {
        match c {
            Codec::Vp8 => Depack::Vp8(Default::default()),
            Codec::Vp9 => Depack::Vp9(Default::default()),
            Codec::H264 => Depack::H264(Default::default()),
            _ => Depack::Opus(Default::default()),
        }
    }
    fn push(&mut self, p: &[u8], m: bool) -> Option<sex_rtp::Frame> {
        match self { Depack::Opus(d) => d.push(p, m), Depack::Vp8(d) => d.push(p, m), Depack::Vp9(d) => d.push(p, m), Depack::H264(d) => d.push(p, m) }
    }
}

pub struct Transceiver {
    pub mid: Option<String>,
    pub kind: Kind,
    pub direction: Direction,
    pub current_direction: Option<Direction>,
    /// Our direction when last agreed (for negotiationneeded).
    pub negotiated: Option<Direction>,
    pub stopped: bool,
    /// m-line position once it has one.
    pub mline: Option<usize>,
    /// Made by addTrack (a remote offer's m-line may take it over).
    pub from_add_track: bool,
    /// What the sender sends: a source the browser knows ("camera", "mic"), or None.
    pub source: Option<String>,
    pub track_id: String,
    pub stream_ids: Vec<String>,
    pub encodings: Vec<Encoding>,
    /// setCodecPreferences, as lowercase mime types.
    pub preferences: Vec<String>,
    /// Negotiated codecs, the sending one first.
    pub codecs: Vec<Negotiated>,
    pub ssrc: u32,
    pub cname: String,
    stream: RtpStream,
    vp8: sex_rtp::codec::Vp8,
    pub remote_ssrc: Option<u32>,
    /// The peer's msid for this m-line: stream ids and track id.
    pub remote_streams: Vec<String>,
    pub remote_track: String,
    jitter: Option<JitterBuffer>,
    depack: Option<(u8, Depack)>,
    rstats: ReceiveStats,
    pub sent: SendStats,
    pub received: RecvStats,
    /// A PLI or FIR asked for a key frame.
    pub want_keyframe: bool,
    pli_at: u64,
    pub encoder: Option<crate::vp8::Encoder>,
    pub next_frame_ms: u64,
    pub audio_buf: Vec<i16>,
    pub audio_ts: u32,
    pub video_ts_base: Option<u64>,
}

impl Transceiver {
    fn new(kind: Kind, direction: Direction, ssrc: u32, seq: u16, cname: &str, track_id: String) -> Transceiver {
        Transceiver {
            mid: None, kind, direction, current_direction: None, negotiated: None, stopped: false, mline: None, from_add_track: false,
            source: None, track_id, stream_ids: Vec::new(), encodings: alloc::vec![Encoding::default()], preferences: Vec::new(),
            codecs: Vec::new(), ssrc, cname: cname.into(), stream: RtpStream::new(ssrc, 0, seq), vp8: Default::default(), remote_ssrc: None,
            remote_streams: Vec::new(), remote_track: String::new(), jitter: None, depack: None, rstats: ReceiveStats::default(),
            sent: SendStats::default(), received: RecvStats::default(), want_keyframe: true, pli_at: 0, encoder: None, next_frame_ms: 0,
            audio_buf: Vec::new(), audio_ts: 0, video_ts_base: None,
        }
    }
    pub fn sending(&self) -> bool { !self.stopped && self.current_direction.is_some_and(|d| d.sends()) && self.source.is_some() }
    pub fn receiving(&self) -> bool { !self.stopped && self.current_direction.is_some_and(|d| d.receives()) }
    pub fn send_codec(&self) -> Option<&Negotiated> { self.codecs.first() }
}

/// A whole frame that arrived on a transceiver, for the browser to decode.
#[derive(Clone, Debug)]
pub struct Received { pub transceiver: usize, pub codec: Codec, pub data: Vec<u8>, pub keyframe: bool, pub timestamp: u32 }

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A local candidate (`candidate:...`); an empty line ends them.
    Candidate { line: String, mid: String },
    Gathering(&'static str),
    Ice(&'static str),
    Connection(&'static str),
    /// Media started arriving on transceiver N (its track unmutes).
    Unmute(usize),
}

pub struct Peer {
    rng: ChaCha20Rng,
    pub certificate: Certificate,
    config: Config,
    pub signaling: Signaling,
    pub transceivers: Vec<Transceiver>,
    session_id: u64,
    version: u64,
    pub local: Option<(SdpType, String)>,
    pub remote: Option<(SdpType, String)>,
    pub current_local: Option<(SdpType, String)>,
    pub current_remote: Option<(SdpType, String)>,
    last_offer: Option<String>,
    last_answer: Option<String>,
    stable_transceivers: usize,
    agent: Option<Agent>,
    pub transport_started: bool,
    wants_transport: bool,
    dtls: Option<Dtls>,
    dtls_role: Option<Role>,
    remote_fp: Option<Fingerprint>,
    remote_ice: Option<(String, String)>,
    remote_lite: bool,
    srtp_tx: Option<sex_srtp::Context>,
    srtp_rx: Option<sex_srtp::Context>,
    pending_candidates: Vec<String>,
    events: VecDeque<Event>,
    out: VecDeque<(Route, Vec<u8>)>,
    frames: VecDeque<Received>,
    pub ice_state: &'static str,
    pub gathering: &'static str,
    pub connection: &'static str,
    pub dtls_state: &'static str,
    pub closed: bool,
    pub bwe: BandwidthEstimator,
    remb_bps: Option<u32>,
    twcc_seq: u16,
    twcc_rx: TwccRecorder,
    ext_abs: Option<u8>,
    ext_twcc: Option<u8>,
    ext_mid: Option<u8>,
    next_rtcp: u64,
    next_nack: u64,
    next_fb: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub packets_sent: u64,
    pub packets_received: u64,
    pub rtt_ms: Option<u32>,
    pub candidates_local: Vec<sex_ice::Candidate>,
    pub candidates_remote: Vec<String>,
    mid_counter: u32,
    pub log: Vec<String>,
}

fn kind_str(k: Kind) -> &'static str { match k { Kind::Audio => "audio", Kind::Video => "video" } }

/// What this side can receive, in the order it prefers.
fn local_codecs(k: Kind) -> &'static [(Codec, u8)] {
    match k {
        Kind::Audio => &[(Codec::Opus, 111), (Codec::Pcmu, 0), (Codec::Pcma, 8)],
        Kind::Video => &[(Codec::Vp8, 96), (Codec::Vp9, 98), (Codec::H264, 102)],
    }
}

fn direction_intersect(ours: Direction, theirs_reversed: Direction) -> Direction {
    let s = ours.sends() && theirs_reversed.sends();
    let r = ours.receives() && theirs_reversed.receives();
    match (s, r) { (true, true) => Direction::SendRecv, (true, false) => Direction::SendOnly, (false, true) => Direction::RecvOnly, _ => Direction::Inactive }
}

pub fn parse_direction(s: &str) -> Option<Direction> {
    Some(match s { "sendrecv" => Direction::SendRecv, "sendonly" => Direction::SendOnly, "recvonly" => Direction::RecvOnly, "inactive" => Direction::Inactive, _ => return None })
}

impl Peer {
    pub fn new(config: Config, seed: [u8; 32]) -> Peer {
        let mut rng = ChaCha20Rng::from_seed(seed);
        let certificate = Certificate::generate(&mut rng);
        let session_id = rng.next_u64() >> 2;
        Peer {
            rng, certificate, config, signaling: Signaling::Stable, transceivers: Vec::new(), session_id, version: 1,
            local: None, remote: None, current_local: None, current_remote: None, last_offer: None, last_answer: None, stable_transceivers: 0,
            agent: None, transport_started: false, wants_transport: false, dtls: None, dtls_role: None, remote_fp: None, remote_ice: None, remote_lite: false,
            srtp_tx: None, srtp_rx: None, pending_candidates: Vec::new(), events: VecDeque::new(), out: VecDeque::new(), frames: VecDeque::new(),
            ice_state: "new", gathering: "new", connection: "new", dtls_state: "new", closed: false,
            bwe: BandwidthEstimator::new(800_000, 50_000, 4_000_000), remb_bps: None, twcc_seq: 0, twcc_rx: TwccRecorder::default(),
            ext_abs: None, ext_twcc: None, ext_mid: None, next_rtcp: 0, next_nack: 0, next_fb: 0,
            bytes_sent: 0, bytes_received: 0, packets_sent: 0, packets_received: 0, rtt_ms: None,
            candidates_local: Vec::new(), candidates_remote: Vec::new(), mid_counter: 0, log: Vec::new(),
        }
    }

    pub fn config(&self) -> &Config { &self.config }
    pub fn set_config(&mut self, c: Config) { self.config = c; }

    fn random_id(&mut self, n: usize) -> String {
        const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
        (0..n).map(|_| A[(self.rng.next_u32() % A.len() as u32) as usize] as char).collect()
    }

    /// addTrack / addTransceiver. Returns the transceiver's index.
    pub fn add_transceiver(&mut self, kind: Kind, direction: Direction, source: Option<String>, track_id: String, stream_ids: Vec<String>, encodings: Vec<Encoding>, from_add_track: bool) -> usize {
        if from_add_track {
            // addTrack reuses a transceiver of its kind that never sent.
            if let Some(i) = self.transceivers.iter().position(|t| t.kind == kind && !t.stopped && t.source.is_none() && !t.from_add_track && t.track_id.is_empty()
                && t.current_direction.map_or(true, |d| !d.sends()) && t.mid.is_some()) {
                let t = &mut self.transceivers[i];
                t.source = source;
                t.track_id = track_id;
                t.stream_ids = stream_ids;
                t.direction = match t.direction { Direction::RecvOnly => Direction::SendRecv, Direction::Inactive => Direction::SendOnly, d => d };
                return i;
            }
        }
        let ssrc = self.rng.next_u32() | 1;
        let seq = self.rng.next_u32() as u16 & 0x7fff;
        let cname = self.random_id(16);
        let mut t = Transceiver::new(kind, direction, ssrc, seq, &cname, track_id);
        t.source = source;
        t.stream_ids = stream_ids;
        t.from_add_track = from_add_track;
        if !encodings.is_empty() { t.encodings = encodings; }
        self.transceivers.push(t);
        self.transceivers.len() - 1
    }

    /// removeTrack: the sender stops sending.
    pub fn remove_track(&mut self, i: usize) {
        if let Some(t) = self.transceivers.get_mut(i) {
            t.source = None;
            t.direction = match t.direction { Direction::SendRecv => Direction::RecvOnly, Direction::SendOnly => Direction::Inactive, d => d };
        }
    }

    pub fn stop_transceiver(&mut self, i: usize) {
        if let Some(t) = self.transceivers.get_mut(i) { t.stopped = true; t.source = None; t.direction = Direction::Inactive; }
    }

    /// Whether a new offer would differ from what was last agreed.
    pub fn negotiation_needed(&self) -> bool {
        if self.closed || self.signaling != Signaling::Stable { return false; }
        self.transceivers.iter().any(|t| if t.stopped { t.negotiated.is_some() } else { t.mid.is_none() || t.mline.is_none() || t.negotiated != Some(t.direction) })
    }

    fn ensure_agent(&mut self, controlling: bool) {
        if self.agent.is_some() { return; }
        let mut seed = [0u8; 32];
        self.rng.fill_bytes(&mut seed);
        let mut c = sex_ice::Config::new(controlling, seed);
        c.relay_only = self.config.relay_only;
        c.software = Some("SexOS Kaleidoscope");
        self.agent = Some(Agent::new(c));
    }

    fn credentials(&self) -> (String, String) {
        let (u, p) = self.agent.as_ref().map(|a| a.local_credentials()).unwrap_or(("", ""));
        (u.into(), p.into())
    }

    fn assign_mids(&mut self) {
        let used: Vec<String> = self.transceivers.iter().filter_map(|t| t.mid.clone()).collect();
        for i in 0..self.transceivers.len() {
            if self.transceivers[i].mid.is_none() && !self.transceivers[i].stopped {
                loop {
                    let m = self.mid_counter.to_string();
                    self.mid_counter += 1;
                    if !used.contains(&m) { self.transceivers[i].mid = Some(m); break; }
                }
            }
        }
    }

    /// The order transceivers appear in a description: kept m-lines first, by position.
    fn mline_order(&self) -> Vec<usize> {
        let mut v: Vec<usize> = (0..self.transceivers.len()).filter(|&i| self.transceivers[i].mline.is_some()).collect();
        v.sort_by_key(|&i| self.transceivers[i].mline);
        for i in 0..self.transceivers.len() {
            if self.transceivers[i].mline.is_none() && !self.transceivers[i].stopped && self.transceivers[i].mid.is_some() { v.push(i); }
        }
        v
    }

    fn codec_list(&self, t: &Transceiver) -> Vec<(Codec, u8)> {
        let mut v: Vec<(Codec, u8)> = local_codecs(t.kind).to_vec();
        if !t.preferences.is_empty() {
            v.retain(|(c, _)| t.preferences.iter().any(|p| *p == c.mime()));
            v.sort_by_key(|(c, _)| t.preferences.iter().position(|p| *p == c.mime()));
        }
        v
    }

    fn media_section(&self, t: &Transceiver, codecs: &[(Codec, u8, Option<String>)], setup: &str, direction: Direction, ext: &[(u8, &str)]) -> String {
        let (ufrag, pwd) = self.credentials();
        let mut s = String::new();
        let kind = kind_str(t.kind);
        if t.stopped || codecs.is_empty() {
            let _ = write!(s, "m={kind} 0 UDP/TLS/RTP/SAVPF {}\r\nc=IN IP4 0.0.0.0\r\n", if t.kind == Kind::Audio { "0" } else { "96" });
            if let Some(m) = &t.mid { let _ = write!(s, "a=mid:{m}\r\n"); }
            s.push_str("a=inactive\r\n");
            return s;
        }
        let pts: Vec<String> = codecs.iter().map(|c| c.1.to_string()).collect();
        let _ = write!(s, "m={kind} 9 UDP/TLS/RTP/SAVPF {}\r\nc=IN IP4 0.0.0.0\r\na=rtcp:9 IN IP4 0.0.0.0\r\n", pts.join(" "));
        let _ = write!(s, "a=ice-ufrag:{ufrag}\r\na=ice-pwd:{pwd}\r\na=ice-options:trickle\r\na=fingerprint:{}\r\na=setup:{setup}\r\n", self.certificate.fingerprint().to_sdp());
        let _ = write!(s, "a=mid:{}\r\n", t.mid.as_deref().unwrap_or(""));
        for (id, uri) in ext { let _ = write!(s, "a=extmap:{id} {uri}\r\n"); }
        let _ = write!(s, "a={}\r\n", direction.as_str());
        let streams = if t.stream_ids.is_empty() { alloc::vec![String::from("-")] } else { t.stream_ids.clone() };
        if direction.sends() {
            for st in &streams { let _ = write!(s, "a=msid:{st} {}\r\n", t.track_id); }
        }
        s.push_str("a=rtcp-mux\r\na=rtcp-rsize\r\n");
        for (c, pt, fmtp) in codecs {
            match c.channels() {
                Some(ch) => { let _ = write!(s, "a=rtpmap:{pt} {}/{}/{ch}\r\n", c.name(), c.clock()); }
                None => { let _ = write!(s, "a=rtpmap:{pt} {}/{}\r\n", c.name(), c.clock()); }
            }
            if t.kind == Kind::Video {
                for fb in ["goog-remb", "transport-cc", "ccm fir", "nack", "nack pli"] { let _ = write!(s, "a=rtcp-fb:{pt} {fb}\r\n"); }
            } else if *c == Codec::Opus {
                let _ = write!(s, "a=rtcp-fb:{pt} transport-cc\r\n");
            }
            let f = fmtp.clone().or_else(|| match c {
                Codec::Opus => Some(String::from("minptime=10;useinbandfec=1")),
                Codec::H264 => Some(String::from("level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f")),
                _ => None,
            });
            if let Some(f) = f { let _ = write!(s, "a=fmtp:{pt} {f}\r\n"); }
        }
        if direction.sends() {
            let _ = write!(s, "a=ssrc:{} cname:{}\r\n", t.ssrc, t.cname);
            for st in &streams { let _ = write!(s, "a=ssrc:{} msid:{st} {}\r\n", t.ssrc, t.track_id); }
        }
        s
    }

    fn session_head(&mut self, mids: &[String]) -> String {
        self.version += 1;
        let mut s = format!("v=0\r\no=- {} {} IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\n", self.session_id, self.version);
        if !mids.is_empty() { let _ = write!(s, "a=group:BUNDLE {}\r\n", mids.join(" ")); }
        s.push_str("a=extmap-allow-mixed\r\na=msid-semantic: WMS *\r\n");
        s
    }

    pub fn create_offer(&mut self, _ice_restart: bool) -> Result<String, Error> {
        if self.closed { return Err(err("InvalidStateError", "The RTCPeerConnection's signalingState is 'closed'.")); }
        self.ensure_agent(true);
        self.assign_mids();
        let order = self.mline_order();
        let mids: Vec<String> = order.iter().filter(|&&i| !self.transceivers[i].stopped).filter_map(|&i| self.transceivers[i].mid.clone()).collect();
        let mut s = self.session_head(&mids);
        let ext: [(u8, &str); 3] = [(1, ABS_SEND_TIME), (3, TWCC), (4, SDES_MID)];
        for &i in &order {
            let t = &self.transceivers[i];
            let codecs: Vec<(Codec, u8, Option<String>)> = self.codec_list(t).into_iter().map(|(c, p)| (c, p, None)).collect();
            s.push_str(&self.media_section(t, &codecs, "actpass", t.direction, &ext));
        }
        self.last_offer = Some(s.clone());
        Ok(s)
    }

    pub fn create_answer(&mut self) -> Result<String, Error> {
        if self.signaling != Signaling::HaveRemoteOffer && self.signaling != Signaling::HaveLocalPranswer {
            return Err(err("InvalidStateError", "Failed to execute 'createAnswer' on 'RTCPeerConnection': PeerConnection cannot create an answer in a state other than have-remote-offer or have-local-pranswer."));
        }
        let remote = Session::parse(&self.remote.as_ref().unwrap().1).map_err(|_| err("InvalidStateError", "no remote offer"))?;
        self.ensure_agent(false);
        let order = self.mline_order();
        let bundle = remote.bundle();
        let mut mids = Vec::new();
        let mut body = String::new();
        for (mi, m) in remote.media.iter().enumerate() {
            let ti = order.iter().copied().find(|&i| self.transceivers[i].mline == Some(mi));
            let Some(ti) = ti else {
                // An m-line we do not take (a data channel): rejected.
                let _ = write!(body, "m={} 0 {} {}\r\nc=IN IP4 0.0.0.0\r\n", m.kind, m.proto, m.formats.first().map_or("0", |f| f.as_str()));
                if let Some(mid) = m.mid() { let _ = write!(body, "a=mid:{mid}\r\n"); }
                continue;
            };
            let t = &self.transceivers[ti];
            let codecs = self.answer_codecs(t, m);
            let dir = direction_intersect(t.direction, m.direction().unwrap_or(Direction::SendRecv).reverse());
            let ext: Vec<(u8, String)> = m.all::<sex_sdp::Extmap>().into_iter().filter(|e| [ABS_SEND_TIME, TWCC, SDES_MID].contains(&e.uri.as_str())).map(|e| (e.id as u8, e.uri)).collect();
            let ext_ref: Vec<(u8, &str)> = ext.iter().map(|(i, u)| (*i, u.as_str())).collect();
            if !t.stopped && !codecs.is_empty() && !m.rejected() && bundle.as_ref().map_or(true, |b| t.mid.as_ref().is_some_and(|x| b.mids.contains(x))) {
                mids.push(t.mid.clone().unwrap_or_default());
            }
            let mut sec = self.media_section(t, &codecs, "active", dir, &ext_ref);
            if m.rejected() { sec = self.media_section(&Transceiver { stopped: true, ..Transceiver::new(t.kind, Direction::Inactive, 0, 0, "", String::new()) }, &[], "", dir, &[]); sec = sec.replacen("a=inactive", &format!("a=mid:{}\r\na=inactive", t.mid.as_deref().unwrap_or("")), 1); }
            body.push_str(&sec);
        }
        let mut s = self.session_head(&mids);
        s.push_str(&body);
        self.last_answer = Some(s.clone());
        Ok(s)
    }

    /// Codecs to answer with for remote m-line `m`: theirs, in their order, that we take,
    /// with their payload types.
    fn answer_codecs(&self, t: &Transceiver, m: &sex_sdp::Media) -> Vec<(Codec, u8, Option<String>)> {
        let ours = self.codec_list(t);
        let mut v = Vec::new();
        for c in m.codecs() {
            let Some(r) = &c.rtpmap else { continue };
            let Some(codec) = Codec::from_rtpmap(&r.encoding, r.clock) else { continue };
            if !ours.iter().any(|(x, _)| *x == codec) || v.iter().any(|(x, _, _): &(Codec, u8, Option<String>)| *x == codec) { continue; }
            if codec == Codec::H264 && !c.fmtp.as_deref().is_some_and(|f| f.contains("packetization-mode=1")) { continue; }
            v.push((codec, c.pt, c.fmtp.clone()));
        }
        // What we can send goes first, so the far side knows what to expect.
        v.sort_by_key(|(c, _, _)| !c.encodable());
        v
    }

    pub fn set_local_description(&mut self, now: u64, ty: SdpType, sdp: &str) -> Result<(), Error> {
        if self.closed { return Err(err("InvalidStateError", "The RTCPeerConnection's signalingState is 'closed'.")); }
        match ty {
            SdpType::Rollback => {
                if self.signaling != Signaling::HaveLocalOffer { return Err(err("InvalidStateError", "Called in wrong state: stable")); }
                self.local = self.current_local.clone();
                self.signaling = Signaling::Stable;
                return Ok(());
            }
            SdpType::Offer => {
                if !matches!(self.signaling, Signaling::Stable | Signaling::HaveLocalOffer) { return Err(err("InvalidModificationError", "Called in wrong state")); }
                let sdp = if sdp.is_empty() { self.last_offer.clone().ok_or_else(|| err("InvalidModificationError", "no offer"))? } else { String::from(sdp) };
                Session::parse(&sdp).map_err(|e| err("OperationError", &e.to_string()))?;
                // m-line positions are now fixed.
                let order = self.mline_order();
                for (pos, &i) in order.iter().enumerate() { self.transceivers[i].mline = Some(pos); }
                self.local = Some((SdpType::Offer, sdp));
                self.signaling = Signaling::HaveLocalOffer;
            }
            SdpType::Answer | SdpType::Pranswer => {
                if !matches!(self.signaling, Signaling::HaveRemoteOffer | Signaling::HaveLocalPranswer) { return Err(err("InvalidStateError", "Called in wrong state: stable")); }
                let sdp = if sdp.is_empty() { self.last_answer.clone().ok_or_else(|| err("InvalidModificationError", "no answer"))? } else { String::from(sdp) };
                let local = Session::parse(&sdp).map_err(|e| err("OperationError", &e.to_string()))?;
                let remote = Session::parse(&self.remote.as_ref().unwrap().1).map_err(|_| err("OperationError", "remote"))?;
                self.local = Some((ty, sdp));
                if ty == SdpType::Answer {
                    self.dtls_role = Some(Role::Client);
                    self.apply_negotiated(&local, &remote, false);
                    self.signaling = Signaling::Stable;
                    self.current_local = self.local.clone();
                    self.current_remote = self.remote.clone();
                } else {
                    self.signaling = Signaling::HaveLocalPranswer;
                }
            }
        }
        self.wants_transport |= !self.transport_started;
        if self.transport_started { if let Some(a) = self.agent.as_mut() { a.start(now); } }
        self.try_start_dtls(now);
        Ok(())
    }

    pub fn set_remote_description(&mut self, now: u64, ty: SdpType, sdp: &str) -> Result<Vec<usize>, Error> {
        if self.closed { return Err(err("InvalidStateError", "The RTCPeerConnection's signalingState is 'closed'.")); }
        if ty == SdpType::Rollback {
            if self.signaling != Signaling::HaveRemoteOffer { return Err(err("InvalidStateError", "Called in wrong state")); }
            self.remote = self.current_remote.clone();
            self.signaling = Signaling::Stable;
            return Ok(Vec::new());
        }
        let s = Session::parse(sdp).map_err(|e| err("OperationError", &format!("Failed to parse SessionDescription. {e}")))?;
        if s.media.is_empty() && ty == SdpType::Offer && false { return Err(err("OperationError", "no media")); }
        // ICE and DTLS parameters from the first bundled m-line.
        let first = s.media.iter().find(|m| !m.rejected());
        if let Some(m) = first {
            let creds = s.ice_credentials(m).ok_or_else(|| err("OperationError", "Failed to set remote description: ICE credentials missing"))?;
            let fp = s.fingerprint(m).filter(|f| f.algorithm == "sha-256" && f.bytes.len() == 32)
                .ok_or_else(|| err("OperationError", "Failed to set remote description: no sha-256 fingerprint"))?;
            let mut b = [0u8; 32];
            b.copy_from_slice(&fp.bytes);
            self.remote_fp = Some(Fingerprint(b));
            self.remote_ice = Some(creds);
            self.remote_lite = s.ice_lite();
        }
        let mut new_tracks = Vec::new();
        match ty {
            SdpType::Offer => {
                if !matches!(self.signaling, Signaling::Stable | Signaling::HaveRemoteOffer) { return Err(err("InvalidStateError", "Failed to set remote offer sdp: Called in wrong state: have-local-offer")); }
                self.ensure_agent(false);
                // Match each m-line to a transceiver: by mid, else one addTrack made.
                for (mi, m) in s.media.iter().enumerate() {
                    let kind = match m.kind.as_str() { "audio" => Kind::Audio, "video" => Kind::Video, _ => continue };
                    let mid = m.mid().map(String::from);
                    let found = self.transceivers.iter().position(|t| t.mid.is_some() && t.mid == mid)
                        .or_else(|| self.transceivers.iter().position(|t| t.kind == kind && t.mid.is_none() && t.mline.is_none() && !t.stopped && t.from_add_track));
                    let i = match found {
                        Some(i) => i,
                        None => {
                            let i = self.add_transceiver(kind, Direction::RecvOnly, None, String::new(), Vec::new(), Vec::new(), false);
                            i
                        }
                    };
                    let t = &mut self.transceivers[i];
                    if t.mid.is_none() { t.mid = mid; }
                    t.mline = Some(mi);
                    let (streams, track) = remote_msid(&s, m);
                    if m.direction().unwrap_or(Direction::SendRecv).sends() && !m.rejected() && (t.remote_track != track || t.remote_track.is_empty()) {
                        t.remote_streams = streams;
                        t.remote_track = if track.is_empty() { format!("{}-{}", kind_str(kind), mi) } else { track };
                        new_tracks.push(i);
                    }
                    if let Some(ss) = m.all::<sex_sdp::Ssrc>().first() { t.remote_ssrc = Some(ss.ssrc); }
                }
                self.remote = Some((ty, sdp.into()));
                self.signaling = Signaling::HaveRemoteOffer;
            }
            SdpType::Answer | SdpType::Pranswer => {
                if !matches!(self.signaling, Signaling::HaveLocalOffer | Signaling::HaveRemotePranswer) { return Err(err("InvalidStateError", "Failed to set remote answer sdp: Called in wrong state: stable")); }
                let local = Session::parse(&self.local.as_ref().unwrap().1).map_err(|_| err("OperationError", "local"))?;
                if let Some(m) = first {
                    self.dtls_role = Some(match s.setup(m) { Some(sex_sdp::Setup::Active) => Role::Server, _ => Role::Client });
                }
                for (mi, m) in s.media.iter().enumerate() {
                    let Some(i) = self.transceivers.iter().position(|t| t.mline == Some(mi)) else { continue };
                    let t = &mut self.transceivers[i];
                    let (streams, track) = remote_msid(&s, m);
                    if m.direction().unwrap_or(Direction::SendRecv).sends() && !m.rejected() && t.remote_track != track {
                        t.remote_streams = streams;
                        t.remote_track = if track.is_empty() { format!("{}-{}", kind_str(t.kind), mi) } else { track };
                        new_tracks.push(i);
                    }
                    if let Some(ss) = m.all::<sex_sdp::Ssrc>().first() { t.remote_ssrc = Some(ss.ssrc); }
                }
                self.remote = Some((ty, sdp.into()));
                if ty == SdpType::Answer {
                    self.apply_negotiated(&local, &s, true);
                    self.signaling = Signaling::Stable;
                    self.current_local = self.local.clone();
                    self.current_remote = self.remote.clone();
                } else {
                    self.signaling = Signaling::HaveRemotePranswer;
                }
            }
            SdpType::Rollback => unreachable!(),
        }
        if self.remote_lite { if let Some(a) = self.agent.as_mut() { a.set_remote_lite(true); } }
        if let (Some(a), Some((u, p))) = (self.agent.as_mut(), self.remote_ice.clone()) { a.set_remote_credentials(&u, &p); }
        // Candidates in the description itself, and ones trickled early.
        let mut lines: Vec<String> = Vec::new();
        for m in &s.media { for c in m.all::<sex_sdp::Candidate>() { lines.push(c.to_line()); } }
        lines.append(&mut self.pending_candidates);
        for l in lines { let _ = self.add_ice_candidate(now, &l); }
        self.try_start_dtls(now);
        Ok(new_tracks)
    }

    /// After an answer: codecs, directions and header extensions in effect.
    fn apply_negotiated(&mut self, local: &Session, remote: &Session, we_offered: bool) {
        let answer = if we_offered { remote } else { local };
        for (mi, m) in answer.media.iter().enumerate() {
            let Some(i) = self.transceivers.iter().position(|t| t.mline == Some(mi)) else { continue };
            let lm = local.media.get(mi);
            let t = &mut self.transceivers[i];
            t.negotiated = if t.stopped { None } else { Some(t.direction) };
            if m.rejected() { t.current_direction = Some(Direction::Inactive); t.negotiated = None; if we_offered { t.stopped = true; } continue; }
            let ldir = if we_offered { m.direction().unwrap_or(Direction::SendRecv).reverse() } else { lm.and_then(|l| l.direction()).unwrap_or(Direction::SendRecv) };
            t.current_direction = Some(ldir);
            // Payload types: the remote side's numbers are what we send with.
            let rm = if we_offered { m } else { remote.media.get(mi).unwrap_or(m) };
            let mut neg = Vec::new();
            for c in m.codecs() {
                let Some(r) = &c.rtpmap else { continue };
                let Some(codec) = Codec::from_rtpmap(&r.encoding, r.clock) else { continue };
                if neg.iter().any(|n: &Negotiated| n.codec == codec) { continue; }
                let pt = rm.codecs().into_iter().find(|x| x.rtpmap.as_ref().and_then(|r| Codec::from_rtpmap(&r.encoding, r.clock)) == Some(codec)).map_or(c.pt, |x| x.pt);
                let fb = |k: &str, p: Option<&str>| c.feedback.iter().any(|f| f.kind == k && f.param.as_deref() == p);
                neg.push(Negotiated { codec, pt, fmtp: c.fmtp.clone(), nack: fb("nack", None), pli: fb("nack", Some("pli")), fir: fb("ccm", Some("fir")), remb: fb("goog-remb", None), twcc: fb("transport-cc", None) });
            }
            neg.sort_by_key(|n| !n.codec.encodable());
            t.codecs = neg;
            if let Some(n) = t.codecs.first() { t.stream.payload_type = n.pt; }
            for e in m.all::<sex_sdp::Extmap>() {
                match e.uri.as_str() {
                    ABS_SEND_TIME => self.ext_abs = Some(e.id as u8),
                    TWCC => self.ext_twcc = Some(e.id as u8),
                    SDES_MID => self.ext_mid = Some(e.id as u8),
                    _ => {}
                }
            }
            let t = &mut self.transceivers[i];
            if t.jitter.is_none() {
                t.jitter = Some(JitterBuffer::new(if t.kind == Kind::Video { JitterConfig::video() } else { JitterConfig::audio() }));
            }
        }
        self.stable_transceivers = self.transceivers.len();
    }

    pub fn add_ice_candidate(&mut self, now: u64, line: &str) -> Result<(), Error> {
        if self.remote.is_none() { return Err(err("InvalidStateError", "Failed to execute 'addIceCandidate' on 'RTCPeerConnection': The remote description was null")); }
        if line.trim().is_empty() {
            if let Some(a) = self.agent.as_mut() { a.end_of_remote_candidates(); }
            return Ok(());
        }
        if sex_sdp::Candidate::parse_line(line).is_none() { return Err(err("OperationError", "Failed to execute 'addIceCandidate' on 'RTCPeerConnection': Error processing ICE candidate")); }
        self.candidates_remote.push(line.into());
        match self.agent.as_mut() {
            Some(a) if self.remote_ice.is_some() => { a.add_remote_candidate_line(now, line); }
            _ => self.pending_candidates.push(line.into()),
        }
        Ok(())
    }

    /// The browser should bind a socket and call `start_transport`.
    pub fn wants_transport(&self) -> bool { self.wants_transport && !self.transport_started && !self.closed }

    /// The socket's addresses (one per machine address, all on its port), and the ICE
    /// servers' resolved addresses (by index in the config; None: could not resolve).
    pub fn start_transport(&mut self, now: u64, hosts: &[SocketAddr], servers: &[Option<SocketAddr>]) -> Vec<usize> {
        self.transport_started = true;
        self.wants_transport = false;
        let Some(a) = self.agent.as_mut() else { return Vec::new() };
        for h in hosts { a.add_host(now, *h); }
        let mut turn = Vec::new();
        for (i, s) in self.config.servers.iter().enumerate() {
            let Some(addr) = servers.get(i).copied().flatten() else { continue };
            match &s.kind {
                ServerKind::Stun => a.add_stun_server(addr),
                ServerKind::Turn(tr) => {
                    let n = a.add_turn_server(sex_ice::turn::Config { server: addr, username: s.username.clone(), password: s.credential.clone(), transport: *tr });
                    turn.push(n);
                }
            }
        }
        a.start(now);
        self.set_gathering("gathering");
        self.pump(now);
        // TURN servers over TCP or TLS want a stream (by agent number).
        turn
    }

    fn set_gathering(&mut self, s: &'static str) { if self.gathering != s { self.gathering = s; self.events.push_back(Event::Gathering(s)); } }
    fn set_ice(&mut self, s: &'static str) { if self.ice_state != s { self.ice_state = s; self.events.push_back(Event::Ice(s)); } }
    fn update_connection(&mut self) {
        let s = if self.closed { "closed" }
            else if self.ice_state == "failed" || self.dtls_state == "failed" { "failed" }
            else if self.ice_state == "disconnected" { "disconnected" }
            else if self.ice_state == "connected" && self.dtls_state == "connected" { "connected" }
            else if self.ice_state == "new" && self.dtls_state == "new" { "new" }
            else { "connecting" };
        if self.connection != s { self.connection = s; self.events.push_back(Event::Connection(s)); }
    }

    fn try_start_dtls(&mut self, now: u64) {
        if self.dtls.is_some() { return; }
        let (Some(role), Some(fp)) = (self.dtls_role, self.remote_fp) else { return };
        let mut seed = [0u8; 32];
        self.rng.fill_bytes(&mut seed);
        let cfg = sex_dtls::Config::new(role, self.certificate.clone(), fp);
        let mut d = Dtls::new(cfg, seed);
        if role == Role::Client && self.ice_state == "connected" { d.start(now); }
        self.dtls = Some(d);
        self.dtls_state = "connecting";
        self.update_connection();
    }

    fn mid_of(&self) -> String {
        self.mline_order().first().and_then(|&i| self.transceivers[i].mid.clone()).unwrap_or_else(|| String::from("0"))
    }

    /// Moves ICE and DTLS along: their events and their datagrams.
    fn pump(&mut self, now: u64) {
        let mut connected_now = false;
        if let Some(a) = self.agent.as_mut() {
            while let Some(t) = a.poll_transmit() { self.out.push_back((t.route, t.data)); }
        }
        loop {
            let Some(e) = self.agent.as_mut().and_then(|a| a.poll_event()) else { break };
            match e {
                sex_ice::Event::Candidate(c) => {
                    let mid = self.mid_of();
                    self.candidates_local.push(c.clone());
                    self.events.push_back(Event::Candidate { line: c.to_sdp().to_line(), mid });
                }
                sex_ice::Event::GatheringDone => {
                    let mid = self.mid_of();
                    self.events.push_back(Event::Candidate { line: String::new(), mid });
                    self.set_gathering("complete");
                }
                sex_ice::Event::State(s) => {
                    let st = match s {
                        sex_ice::State::New => "new", sex_ice::State::Checking => "checking", sex_ice::State::Connected => "connected",
                        sex_ice::State::Disconnected => "disconnected", sex_ice::State::Failed => "failed", sex_ice::State::Closed => "closed",
                    };
                    if st == "connected" { connected_now = true; }
                    self.set_ice(st);
                    self.update_connection();
                }
                sex_ice::Event::Selected { local, remote } => { self.log.push(format!("selected {local} -> {remote}")); }
                sex_ice::Event::TurnFailed { server, code, reason } => { self.log.push(format!("turn {server} failed {code} {reason}")); }
            }
        }
        if connected_now {
            if let Some(d) = self.dtls.as_mut() { if d.role() == Role::Client { d.start(now); } }
        }
        // DTLS records go out once a pair is selected.
        let selected = self.agent.as_ref().is_some_and(|a| a.selected().is_some());
        if selected {
            while let Some(rec) = self.dtls.as_mut().and_then(|d| d.poll_transmit()) {
                if let Some(t) = self.agent.as_mut().and_then(|a| a.send(&rec)) { self.out.push_back((t.route, t.data)); }
            }
        }
        while let Some(e) = self.dtls.as_mut().and_then(|d| d.poll_event()) {
            match e {
                sex_dtls::Event::Connected => {
                    if let Some(k) = self.dtls.as_ref().and_then(|d| d.srtp_keys()) {
                        let (tx, rx) = k.contexts();
                        self.srtp_tx = Some(tx);
                        self.srtp_rx = Some(rx);
                    }
                    self.dtls_state = "connected";
                    self.update_connection();
                    for t in self.transceivers.iter_mut() { t.want_keyframe = true; }
                }
                sex_dtls::Event::Closed => { self.dtls_state = "closed"; self.update_connection(); }
                sex_dtls::Event::Data(_) => {} // SCTP: no data channels yet
            }
        }
    }

    /// A datagram to the socket bound at `local` from `from`.
    pub fn receive(&mut self, now: u64, local: SocketAddr, from: SocketAddr, data: &[u8]) {
        let Some(a) = self.agent.as_mut() else { return };
        self.bytes_received += data.len() as u64;
        match a.receive_udp(now, local, from, data) {
            sex_ice::Received::Ice => {}
            sex_ice::Received::Data { data, .. } => { let d = data.into_owned(); self.receive_data(now, &d); }
        }
        self.pump(now);
    }

    /// Bytes from the stream to TURN server `server` (TCP or TLS).
    pub fn receive_stream(&mut self, now: u64, server: usize, bytes: &[u8]) {
        let Some(a) = self.agent.as_mut() else { return };
        for (_, d) in a.receive_stream(now, server, bytes) { self.receive_data(now, &d); }
        self.pump(now);
    }

    fn receive_data(&mut self, now: u64, d: &[u8]) {
        let Some(&b) = d.first() else { return };
        if (20..=63).contains(&b) {
            if let Some(dt) = self.dtls.as_mut() {
                if let Err(e) = dt.handle_datagram(now, d) {
                    self.log.push(format!("dtls {e:?}"));
                    self.dtls_state = "failed";
                    self.update_connection();
                }
            }
        } else if sex_rtp::is_rtp_or_rtcp(b) {
            if sex_rtp::is_rtcp(d) { self.receive_rtcp(now, d); } else { self.receive_rtp(now, d); }
        }
    }

    fn receive_rtp(&mut self, now: u64, d: &[u8]) {
        let Some(rx) = self.srtp_rx.as_mut() else { return };
        let Ok(plain) = rx.unprotect_rtp(d) else { return };
        let Ok(p) = Packet::parse(&plain) else { return };
        self.packets_received += 1;
        if let Some(id) = self.ext_twcc { if let Some(s) = p.extension(id).and_then(sex_rtp::packet::twcc_seq) { self.twcc_rx.record(s, now * 1000); } }
        // Which transceiver: by SSRC, by the mid extension, then by payload type.
        let mid = self.ext_mid.and_then(|id| p.extension(id)).and_then(|m| core::str::from_utf8(m).ok()).map(String::from);
        let ti = self.transceivers.iter().position(|t| t.remote_ssrc == Some(p.ssrc) && t.receiving())
            .or_else(|| mid.as_ref().and_then(|m| self.transceivers.iter().position(|t| t.mid.as_ref() == Some(m) && t.receiving())))
            .or_else(|| self.transceivers.iter().position(|t| t.receiving() && t.remote_ssrc.is_none() && t.codecs.iter().any(|c| c.pt == p.payload_type)));
        let Some(ti) = ti else { return };
        let t = &mut self.transceivers[ti];
        if t.remote_ssrc.is_none() { t.remote_ssrc = Some(p.ssrc); }
        if t.remote_ssrc != Some(p.ssrc) { return; }
        let Some(codec) = t.codecs.iter().find(|c| c.pt == p.payload_type).map(|c| c.codec) else { return };
        let first = t.received.packets == 0;
        t.received.packets += 1;
        t.received.bytes += p.payload.len() as u64;
        t.received.header_bytes += (plain.len() - p.payload.len()) as u64;
        t.received.last_packet_ms = now;
        let clock = codec.clock();
        t.rstats.packet(p.seq, p.timestamp, (now * clock as u64 / 1000) as u32);
        if first { self.events.push_back(Event::Unmute(ti)); }
        let t = &mut self.transceivers[ti];
        let pt = p.payload_type;
        if t.depack.as_ref().map_or(true, |d| d.0 != pt) { t.depack = Some((pt, Depack::new(codec))); }
        let Some(jb) = t.jitter.as_mut() else { return };
        jb.push(p, now);
        self.drain_frames(ti, now);
    }

    /// Whole frames out of transceiver `ti`'s jitter buffer.
    fn drain_frames(&mut self, ti: usize, now: u64) {
        let t = &mut self.transceivers[ti];
        let (Some(jb), Some((pt, dp))) = (t.jitter.as_mut(), t.depack.as_mut()) else { return };
        let Some(codec) = t.codecs.iter().find(|c| c.pt == *pt).map(|c| c.codec) else { return };
        while let Some(pkts) = jb.pop(now) {
            let ts = pkts.last().map_or(0, |p| p.timestamp);
            let mut got = None;
            for p in &pkts { if let Some(f) = dp.push(&p.payload, p.marker) { got = Some(f); } }
            if let Some(f) = got {
                t.received.frames += 1;
                if f.keyframe { t.received.keyframes += 1; }
                self.frames.push_back(Received { transceiver: ti, codec, data: f.data, keyframe: f.keyframe, timestamp: ts });
            }
        }
    }

    fn receive_rtcp(&mut self, now: u64, d: &[u8]) {
        let Some(rx) = self.srtp_rx.as_mut() else { return };
        let Ok(plain) = rx.unprotect_rtcp(d) else { return };
        let Ok(list) = Rtcp::parse_compound(&plain) else { return };
        for r in list {
            match r {
                Rtcp::Nack { media_ssrc, pairs, .. } => {
                    let Some(ti) = self.transceivers.iter().position(|t| t.ssrc == media_ssrc) else { continue };
                    self.transceivers[ti].sent.nacks += 1;
                    for seq in sex_rtp::rtcp::nack_seqs(&pairs) {
                        let Some(p) = self.transceivers[ti].stream.resend(seq).cloned() else { continue };
                        self.transceivers[ti].sent.retransmitted_packets += 1;
                        self.transceivers[ti].sent.retransmitted_bytes += p.payload.len() as u64;
                        self.send_packet(now, p, false);
                    }
                }
                Rtcp::Pli { media_ssrc, .. } => {
                    if let Some(t) = self.transceivers.iter_mut().find(|t| t.ssrc == media_ssrc) { t.sent.plis += 1; t.want_keyframe = true; }
                }
                Rtcp::Fir { entries, .. } => {
                    for (ssrc, _) in entries { if let Some(t) = self.transceivers.iter_mut().find(|t| t.ssrc == ssrc) { t.sent.firs += 1; t.want_keyframe = true; } }
                }
                Rtcp::Remb { bitrate, .. } => self.remb_bps = Some(bitrate.min(u32::MAX as u64) as u32),
                Rtcp::TransportCc(fb) => self.bwe.on_feedback(&fb, (now * 1000) as i64),
                Rtcp::SenderReport { ssrc, ntp, reports, .. } => {
                    if let Some(t) = self.transceivers.iter_mut().find(|t| t.remote_ssrc == Some(ssrc)) { t.rstats.sender_report(ntp, now); }
                    self.reports(now, &reports);
                }
                Rtcp::ReceiverReport { reports, .. } => self.reports(now, &reports),
                _ => {}
            }
        }
    }

    /// Round-trip time from report blocks about our streams (RFC 3550 6.4.1).
    fn reports(&mut self, now: u64, blocks: &[sex_rtp::ReportBlock]) {
        for b in blocks {
            if b.last_sr == 0 || !self.transceivers.iter().any(|t| t.ssrc == b.ssrc) { continue; }
            let a = (ntp(now) >> 16) as u32;
            let rtt = a.wrapping_sub(b.last_sr).wrapping_sub(b.delay_since_last_sr);
            let ms = (rtt as u64 * 1000 / 65536) as u32;
            if ms < 60_000 {
                self.rtt_ms = Some(ms);
                self.bwe.rtt_ms = ms.max(1);
                for t in self.transceivers.iter_mut() { if let Some(j) = t.jitter.as_mut() { j.rtt_ms = ms.max(5) as u64; } }
            }
        }
    }

    fn send_packet(&mut self, now: u64, mut p: Packet, fresh: bool) {
        let Some(tx) = self.srtp_tx.as_mut() else { return };
        if let Some(id) = self.ext_abs { p.set_extension(id, sex_rtp::packet::abs_send_time(now * 1000).to_vec()); }
        if let Some(id) = self.ext_twcc {
            let s = self.twcc_seq;
            self.twcc_seq = s.wrapping_add(1);
            p.set_extension(id, s.to_be_bytes().to_vec());
            self.bwe.on_sent(s, p.payload.len() + 40, (now * 1000) as i64);
        }
        let raw = p.write();
        let Ok(w) = tx.protect_rtp(&raw) else { return };
        let _ = fresh;
        self.transmit(&w);
    }

    fn transmit(&mut self, data: &[u8]) {
        let Some(a) = self.agent.as_mut() else { return };
        if let Some(t) = a.send(data) {
            self.bytes_sent += data.len() as u64;
            self.packets_sent += 1;
            self.out.push_back((t.route, t.data));
        }
    }

    fn send_rtcp(&mut self, packets: &[Rtcp]) {
        if packets.is_empty() { return; }
        let Some(tx) = self.srtp_tx.as_mut() else { return };
        let Ok(w) = tx.protect_rtcp(&Rtcp::compound(packets)) else { return };
        self.transmit(&w);
    }

    /// Whether media can flow now.
    pub fn media_ready(&self) -> bool { self.srtp_tx.is_some() && self.agent.as_ref().is_some_and(|a| a.selected().is_some()) }

    /// The bitrate the video encoder should aim at for transceiver `i`.
    pub fn target_bps(&self, i: usize) -> u32 {
        let mut b = self.bwe.target_bps();
        if let Some(r) = self.remb_bps { b = b.min(r.max(30_000)); }
        let audio = self.transceivers.iter().filter(|t| t.kind == Kind::Audio && t.sending()).count() as u32 * 64_000;
        b = b.saturating_sub(audio).max(50_000);
        let videos = self.transceivers.iter().filter(|t| t.kind == Kind::Video && t.sending()).count().max(1) as u32;
        b /= videos;
        if let Some(m) = self.transceivers.get(i).and_then(|t| t.encodings.iter().find(|e| e.active)).and_then(|e| e.max_bitrate) { b = b.min(m.max(30_000)); }
        b
    }

    /// One encoded frame on transceiver `i`: VP8 at 90 kHz, or one audio packet at the
    /// codec's clock.
    pub fn send_frame(&mut self, now: u64, i: usize, data: &[u8], keyframe: bool, timestamp: u32) {
        let Some(t) = self.transceivers.get_mut(i) else { return };
        let Some(codec) = t.send_codec().map(|c| c.codec) else { return };
        let mtu = MTU - 12 - 30 - 10;
        let payloads = match codec {
            Codec::Vp8 => t.vp8.packetize(data, mtu),
            _ => alloc::vec![data.to_vec()],
        };
        let mut pkts = t.stream.packets(payloads, timestamp);
        t.sent.frames += 1;
        if keyframe { t.sent.keyframes += 1; }
        let mid = t.mid.clone();
        for p in pkts.iter_mut() {
            t.sent.packets += 1;
            t.sent.bytes += p.payload.len() as u64;
            t.sent.header_bytes += 12;
            if codec.is_audio() { p.marker = false; }
        }
        if codec.is_audio() && t.sent.packets == 1 { if let Some(p) = pkts.first_mut() { p.marker = true; } }
        for mut p in pkts {
            if let (Some(id), Some(m)) = (self.ext_mid, mid.as_ref()) { p.set_extension(id, m.as_bytes().to_vec()); }
            self.send_packet(now, p, true);
        }
    }

    /// A decoder could not use what came: ask the sender for a key frame.
    pub fn request_keyframe(&mut self, now: u64, i: usize) {
        let Some(t) = self.transceivers.get_mut(i) else { return };
        let Some(media) = t.remote_ssrc else { return };
        if now < t.pli_at + 300 { return; }
        t.pli_at = now;
        t.received.plis += 1;
        let sender = t.ssrc;
        self.send_rtcp(&[Rtcp::Pli { sender_ssrc: sender, media_ssrc: media }]);
    }

    pub fn poll_frame(&mut self) -> Option<Received> { self.frames.pop_front() }
    pub fn poll_event(&mut self) -> Option<Event> { self.events.pop_front() }
    /// A datagram to send: from the host socket at `from` to `to`, or on TURN stream N.
    pub fn poll_transmit(&mut self) -> Option<(Route, Vec<u8>)> { self.out.pop_front() }

    pub fn poll_timeout(&self) -> Option<u64> {
        let mut t = self.agent.as_ref().and_then(|a| a.poll_timeout());
        let d = self.dtls.as_ref().and_then(|d| d.poll_timeout());
        t = match (t, d) { (Some(a), Some(b)) => Some(a.min(b)), (a, b) => a.or(b) };
        if self.media_ready() { t = Some(t.map_or(self.next_nack, |x| x.min(self.next_nack))); }
        t
    }

    pub fn handle_timeout(&mut self, now: u64) {
        if self.closed { return; }
        if let Some(a) = self.agent.as_mut() { a.handle_timeout(now); }
        if let Some(d) = self.dtls.as_mut() {
            if d.handle_timeout(now).is_err() { self.dtls_state = "failed"; self.update_connection(); }
        }
        if self.media_ready() {
            if now >= self.next_nack {
                self.next_nack = now + 20;
                let mut out = Vec::new();
                for t in self.transceivers.iter_mut() {
                    let (Some(j), Some(media)) = (t.jitter.as_mut(), t.remote_ssrc) else { continue };
                    let seqs = j.nacks(now);
                    if !seqs.is_empty() && t.codecs.iter().any(|c| c.nack) {
                        t.received.nacks += 1;
                        out.push(Rtcp::Nack { sender_ssrc: t.ssrc, media_ssrc: media, pairs: sex_rtp::rtcp::nack_pairs(&seqs) });
                    }
                    // Frames given up on need a key frame to recover.
                    if j.lost_frames > 0 && now >= t.pli_at + 300 {
                        j.lost_frames = 0;
                        t.pli_at = now;
                        t.received.plis += 1;
                        out.push(Rtcp::Pli { sender_ssrc: t.ssrc, media_ssrc: media });
                    }
                }
                for ti in 0..self.transceivers.len() { self.drain_frames(ti, now); }
                if now >= self.next_fb && self.ext_twcc.is_some() {
                    self.next_fb = now + 100;
                    let sender = self.transceivers.first().map_or(1, |t| t.ssrc);
                    let media = self.transceivers.iter().find_map(|t| t.remote_ssrc).unwrap_or(0);
                    if let Some(fb) = self.twcc_rx.feedback(sender, media) { out.push(Rtcp::TransportCc(fb)); }
                }
                self.send_rtcp(&out);
            }
            if now >= self.next_rtcp {
                self.next_rtcp = now + 1000;
                let mut out = Vec::new();
                let n = ntp(now);
                for t in self.transceivers.iter_mut() {
                    let mut reports = Vec::new();
                    if let Some(r) = t.remote_ssrc { if t.received.packets > 0 { reports.push(t.rstats.report(r, now)); } }
                    if t.sent.packets > 0 {
                        let clock = t.send_codec().map_or(90_000, |c| c.codec.clock()) as u64;
                        let rtp_ts = match t.video_ts_base { Some(_) => (now * clock / 1000) as u32, None => t.audio_ts };
                        if let Rtcp::SenderReport { ssrc, ntp, rtp_ts, packets, octets, .. } = t.stream.sender_report(n, rtp_ts) {
                            out.push(Rtcp::SenderReport { ssrc, ntp, rtp_ts, packets, octets, reports });
                        }
                    } else if !reports.is_empty() {
                        out.push(Rtcp::ReceiverReport { ssrc: t.ssrc, reports });
                    }
                    if t.sent.packets > 0 { out.push(Rtcp::Sdes(alloc::vec![(t.ssrc, t.cname.clone())])); }
                }
                self.send_rtcp(&out);
            }
        }
        self.pump(now);
    }

    pub fn close(&mut self, now: u64) {
        if self.closed { return; }
        let byes: Vec<u32> = self.transceivers.iter().filter(|t| t.sent.packets > 0).map(|t| t.ssrc).collect();
        if !byes.is_empty() { self.send_rtcp(&[Rtcp::Bye { ssrcs: byes, reason: None }]); }
        if let Some(d) = self.dtls.as_mut() { d.close(); }
        self.pump(now);
        if let Some(a) = self.agent.as_mut() { a.close(now); while let Some(t) = a.poll_transmit() { self.out.push_back((t.route, t.data)); } }
        self.closed = true;
        self.signaling = Signaling::Closed;
        for t in self.transceivers.iter_mut() { t.stopped = true; t.current_direction = None; t.source = None; }
        self.ice_state = "closed";
        self.connection = "closed";
    }

    /// The description `localDescription` shows: with the candidates found so far.
    pub fn local_with_candidates(&self) -> Option<(SdpType, String)> {
        let (ty, sdp) = self.local.clone()?;
        if self.candidates_local.is_empty() { return Some((ty, sdp)); }
        let mut out = String::new();
        let mut first = true;
        let lines: Vec<String> = self.candidates_local.iter().map(|c| format!("a={}\r\n", c.to_sdp().to_line())).collect();
        let done = self.gathering == "complete";
        for l in sdp.split_inclusive('\n') {
            out.push_str(l);
            if l.starts_with("a=mid:") && first {
                for c in &lines { out.push_str(c); }
                if done { out.push_str("a=end-of-candidates\r\n"); }
                first = false;
            }
        }
        // Use the real port of the first candidate on the m-line, as browsers do.
        Some((ty, out))
    }

    /// The selected candidate pair, for stats.
    pub fn selected(&self) -> Option<(SocketAddr, SocketAddr)> { self.agent.as_ref().and_then(|a| a.selected()) }
}

fn remote_msid(s: &Session, m: &sex_sdp::Media) -> (Vec<String>, String) {
    let msids: Vec<sex_sdp::Msid> = m.all();
    if !msids.is_empty() {
        let track = msids[0].track.clone().unwrap_or_default();
        return (msids.into_iter().map(|x| x.stream).filter(|x| x != "-").collect(), track);
    }
    // Plan B style: a=ssrc:N msid:stream track.
    for ss in m.all::<sex_sdp::Ssrc>() {
        if ss.attribute == "msid" {
            let v = ss.value.unwrap_or_default();
            let mut w = v.split_ascii_whitespace();
            let st = w.next().unwrap_or("").to_string();
            let tr = w.next().unwrap_or("").to_string();
            return (if st.is_empty() || st == "-" { Vec::new() } else { alloc::vec![st] }, tr);
        }
    }
    let _ = s;
    (Vec::new(), String::new())
}

/// NTP time for `now` ms (the epoch does not matter: both ends only take differences).
fn ntp(now: u64) -> u64 { ((now / 1000) << 32) | (((now % 1000) << 32) / 1000) }

pub fn ip_of(a: &SocketAddr) -> IpAddr { a.ip() }
