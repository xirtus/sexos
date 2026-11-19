// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! AVDTP 1.3 signaling (on L2CAP PSM 0x0019) for an A2DP Source: our one stream end
//! point (SEID 1, audio, source, SBC) and a state machine that sets up a stream to a
//! sink, as initiator (DISCOVER, GET_CAPABILITIES, SET_CONFIGURATION, OPEN, START,
//! SUSPEND, CLOSE, ABORT) or as acceptor when the headset sets it up itself.
//!
//! The transport (media) channel is a second L2CAP channel on the same PSM; the host
//! opens it after OPEN is accepted, or accepts the peer's, and calls
//! [`Source::media_opened`]. Messages are reassembled from start/continue/end packets
//! with bounded size; every parameter from the peer is length-checked, and a malformed
//! command gets a reject, never a panic.

use crate::sbc::{self, Allocation, Mode};
use alloc::vec;
use alloc::vec::Vec;

pub const PSM: u16 = 0x0019;

/// Signal identifiers.
pub mod signal {
    pub const DISCOVER: u8 = 0x01;
    pub const GET_CAPABILITIES: u8 = 0x02;
    pub const SET_CONFIGURATION: u8 = 0x03;
    pub const GET_CONFIGURATION: u8 = 0x04;
    pub const RECONFIGURE: u8 = 0x05;
    pub const OPEN: u8 = 0x06;
    pub const START: u8 = 0x07;
    pub const CLOSE: u8 = 0x08;
    pub const SUSPEND: u8 = 0x09;
    pub const ABORT: u8 = 0x0A;
    pub const SECURITY_CONTROL: u8 = 0x0B;
    pub const GET_ALL_CAPABILITIES: u8 = 0x0C;
    pub const DELAY_REPORT: u8 = 0x0D;
}

/// Message types.
pub mod kind {
    pub const COMMAND: u8 = 0;
    pub const GENERAL_REJECT: u8 = 1;
    pub const ACCEPT: u8 = 2;
    pub const REJECT: u8 = 3;
}

/// Error codes.
pub mod error {
    pub const BAD_HEADER_FORMAT: u8 = 0x01;
    pub const BAD_LENGTH: u8 = 0x11;
    pub const BAD_ACP_SEID: u8 = 0x12;
    pub const SEP_IN_USE: u8 = 0x13;
    pub const SEP_NOT_IN_USE: u8 = 0x14;
    pub const BAD_SERV_CATEGORY: u8 = 0x17;
    pub const BAD_PAYLOAD_FORMAT: u8 = 0x18;
    pub const NOT_SUPPORTED_COMMAND: u8 = 0x19;
    pub const INVALID_CAPABILITIES: u8 = 0x1A;
    pub const UNSUPPORTED_CONFIGURATION: u8 = 0x29;
    pub const BAD_STATE: u8 = 0x31;
    /// Ours: no audio sink end point we can use.
    pub const NO_SINK: u8 = 0xF0;
    /// Ours: the sink offers no SBC configuration we can stream (48 kHz).
    pub const NO_CONFIGURATION: u8 = 0xF1;
}

/// Service categories.
pub mod category {
    pub const MEDIA_TRANSPORT: u8 = 0x01;
    pub const REPORTING: u8 = 0x02;
    pub const RECOVERY: u8 = 0x03;
    pub const CONTENT_PROTECTION: u8 = 0x04;
    pub const HEADER_COMPRESSION: u8 = 0x05;
    pub const MULTIPLEXING: u8 = 0x06;
    pub const MEDIA_CODEC: u8 = 0x07;
    pub const DELAY_REPORTING: u8 = 0x08;
}

pub const MEDIA_AUDIO: u8 = 0x00;
pub const CODEC_SBC: u8 = 0x00;
/// Our stream end point.
pub const LOCAL_SEID: u8 = 1;
/// The largest reassembled message we accept.
pub const MAX_MESSAGE: usize = 2048;
/// Sink end points we look at.
const MAX_SEPS: usize = 32;

/// The packets carrying one message, each at most `mtu` bytes (start, continue and end
/// packets when it does not fit one).
pub fn packets(label: u8, kind: u8, signal: u8, params: &[u8], mtu: usize) -> Vec<Vec<u8>> {
    let mtu = mtu.max(8);
    let h = (label & 0x0F) << 4 | (kind & 3);
    if params.len() + 2 <= mtu {
        let mut p = vec![h, signal & 0x3F];
        p.extend_from_slice(params);
        return vec![p];
    }
    let first = mtu - 3;
    let rest = params.len() - first;
    let n = 1 + rest.div_ceil(mtu - 1);
    let mut out = Vec::with_capacity(n);
    let mut p = vec![h | 1 << 2, n.min(255) as u8, signal & 0x3F];
    p.extend_from_slice(&params[..first]);
    out.push(p);
    let chunks: Vec<&[u8]> = params[first..].chunks(mtu - 1).collect();
    for (i, c) in chunks.iter().enumerate() {
        let t = if i + 1 == chunks.len() { 3 } else { 2 };
        let mut p = vec![h | t << 2];
        p.extend_from_slice(c);
        out.push(p);
    }
    out
}

/// One whole signaling message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message { pub label: u8, pub kind: u8, pub signal: u8, pub params: Vec<u8> }

/// Joins start/continue/end packets.
#[derive(Default)]
pub struct Reassembly { cur: Option<(Message, u8)> }

impl Reassembly {
    pub fn push(&mut self, p: &[u8]) -> Option<Message> {
        let &h = p.first()?;
        let (label, packet, kind) = (h >> 4, h >> 2 & 3, h & 3);
        match packet {
            0 => { self.cur = None; Some(Message { label, kind, signal: *p.get(1)? & 0x3F, params: p[2..].to_vec() }) }
            1 => {
                let (nosp, signal) = (*p.get(1)?, *p.get(2)? & 0x3F);
                self.cur = (nosp >= 2).then(|| (Message { label, kind, signal, params: p[3..].to_vec() }, nosp - 1));
                None
            }
            _ => {
                let (mut m, left) = self.cur.take()?;
                if m.label != label || m.kind != kind || m.params.len() + p.len() - 1 > MAX_MESSAGE { return None; }
                m.params.extend_from_slice(&p[1..]);
                let left = left - 1;
                match (packet, left) {
                    (3, 0) => Some(m),
                    (2, l) if l > 0 => { self.cur = Some((m, l)); None }
                    _ => None, // the count and the packet types disagree: drop it
                }
            }
        }
    }
}

/// A remote stream end point from DISCOVER.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sep { pub seid: u8, pub in_use: bool, pub media: u8, pub sink: bool }

pub fn parse_seps(p: &[u8]) -> Vec<Sep> {
    p.chunks_exact(2).take(MAX_SEPS).map(|c| Sep { seid: c[0] >> 2, in_use: c[0] & 2 != 0, media: c[1] >> 4, sink: c[1] & 8 != 0 })
        .filter(|s| (1..=0x3E).contains(&s.seid)).collect()
}

/// The service capabilities in `p` as (category, data), or None when a length runs past
/// the end.
pub fn parse_capabilities(p: &[u8]) -> Option<Vec<(u8, &[u8])>> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < p.len() {
        let cat = p[at];
        let len = *p.get(at + 1)? as usize;
        out.push((cat, p.get(at + 2..at + 2 + len)?));
        at += 2 + len;
        if out.len() > 16 { return None; }
    }
    Some(out)
}

/// The SBC codec information element (A2DP 4.3.2): bit masks of what is supported,
/// or (in a configuration) exactly one bit of each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SbcInfo(pub [u8; 4]);

/// Our own capabilities: 48 kHz (Bell's rate) in every mode, blocks, subbands and
/// allocation, bitpool 2..=53.
pub const OUR_SBC: SbcInfo = SbcInfo([0x10 | 0x0F, 0xFF, 2, 53]);
/// The bitpool we use at most (A2DP's high-quality joint stereo setting).
pub const BITPOOL: u8 = 53;

impl SbcInfo {
    /// The SBC info inside a media codec capability.
    pub fn from_codec(data: &[u8]) -> Option<SbcInfo> {
        if data.len() < 6 || data[0] >> 4 != MEDIA_AUDIO || data[1] != CODEC_SBC { return None; }
        Some(SbcInfo(data[2..6].try_into().ok()?))
    }
    pub fn codec(&self) -> [u8; 6] { let i = self.0; [MEDIA_AUDIO << 4, CODEC_SBC, i[0], i[1], i[2], i[3]] }

    /// The best configuration we can stream within these capabilities: 48 kHz, then
    /// joint stereo, 16 blocks, 8 subbands, loudness where offered.
    pub fn choose(&self) -> Option<(sbc::Config, SbcInfo)> {
        let [a, b, lo, hi] = self.0;
        if a & 0x10 == 0 { return None; }
        let pick = |bits: u8, order: &[u8]| order.iter().copied().find(|o| bits & o != 0);
        let mode_bit = pick(a & 0x0F, &[0x01, 0x02, 0x04, 0x08])?;
        let block_bit = pick(b & 0xF0, &[0x10, 0x20, 0x40, 0x80])?;
        let sub_bit = pick(b & 0x0C, &[0x04, 0x08])?;
        let alloc_bit = pick(b & 0x03, &[0x01, 0x02])?;
        let chosen = SbcInfo([0x10 | mode_bit, block_bit | sub_bit | alloc_bit, lo, hi]);
        chosen.config().map(|c| (c, SbcInfo([chosen.0[0], chosen.0[1], lo.max(2), c.bitpool])))
    }

    /// The stream configuration this (single-choice) element names, with the bitpool
    /// we use inside its range; None when it is not one choice each, not 48 kHz, or the
    /// range is empty.
    pub fn config(&self) -> Option<sbc::Config> {
        let [a, b, lo, hi] = self.0;
        let one = |v: u8| v != 0 && v & (v - 1) == 0;
        if a & 0xF0 != 0x10 || !one(a & 0x0F) || !one(b & 0xF0) || !one(b & 0x0C) || !one(b & 0x03) { return None; }
        let mode = match a & 0x0F { 0x08 => Mode::Mono, 0x04 => Mode::DualChannel, 0x02 => Mode::Stereo, _ => Mode::JointStereo };
        let blocks = match b & 0xF0 { 0x80 => 4, 0x40 => 8, 0x20 => 12, _ => 16 };
        let subbands = if b & 0x0C == 0x08 { 4 } else { 8 };
        let allocation = if b & 0x03 == 0x02 { Allocation::Snr } else { Allocation::Loudness };
        let mut c = sbc::Config { rate: 48_000, blocks, mode, allocation, subbands, bitpool: 2 };
        let top = hi.min(BITPOOL).min(c.max_bitpool());
        if top < lo.max(2) { return None; }
        c.bitpool = top;
        c.valid().then_some(c)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Discovering,
    Capabilities,
    Configuring,
    Configured,
    /// OPEN sent or accepted: waiting for the transport channel.
    Opening,
    Open,
    Starting,
    Streaming,
    Suspending,
    Closing,
    Aborting,
}

/// What the host should do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Out {
    /// Send this packet on the signaling channel.
    Send(Vec<u8>),
    /// Open the transport channel (a second L2CAP channel to the AVDTP PSM).
    OpenMedia,
    /// The stream is configured with this SBC coding.
    Configured(sbc::Config),
    Streaming,
    Suspended,
    /// The stream is gone (closed or aborted): drop the transport channel.
    Closed,
    /// Setup failed with this AVDTP error code (or one of ours).
    Failed(u8),
}

/// Our A2DP Source end point and the one stream it may have.
pub struct Source {
    pub state: State,
    /// The sink's SEID.
    pub remote: u8,
    pub config: Option<sbc::Config>,
    /// We set the stream up (else the headset did).
    pub initiator: bool,
    label: u8,
    pending: Option<(u8, u8)>,
    candidates: Vec<u8>,
    mtu: usize,
    rx: Reassembly,
}

impl Source {
    /// `mtu`: the signaling channel's outgoing MTU.
    pub fn new(mtu: usize) -> Source {
        Source { state: State::Idle, remote: 0, config: None, initiator: false, label: 0, pending: None, candidates: Vec::new(), mtu, rx: Reassembly::default() }
    }

    fn command(&mut self, signal: u8, params: &[u8], out: &mut Vec<Out>) {
        self.label = (self.label + 1) & 0x0F;
        self.pending = Some((self.label, signal));
        for p in packets(self.label, kind::COMMAND, signal, params, self.mtu) { out.push(Out::Send(p)); }
    }
    fn reply(&self, label: u8, kind: u8, signal: u8, params: &[u8], out: &mut Vec<Out>) {
        for p in packets(label, kind, signal, params, self.mtu) { out.push(Out::Send(p)); }
    }

    /// Start setting a stream up (we are initiator): DISCOVER.
    pub fn start(&mut self) -> Vec<Out> {
        let mut out = Vec::new();
        if self.state == State::Idle {
            self.initiator = true;
            self.state = State::Discovering;
            self.command(signal::DISCOVER, &[], &mut out);
        }
        out
    }

    /// The transport channel is open: the stream is open. When we set it up we start it;
    /// a headset that set it up starts it itself (or the host calls [`Source::resume`]).
    pub fn media_opened(&mut self) -> Vec<Out> {
        let mut out = Vec::new();
        if self.state == State::Opening {
            self.state = State::Open;
            if self.initiator { self.resume_into(&mut out); }
        }
        out
    }

    fn resume_into(&mut self, out: &mut Vec<Out>) {
        if self.state == State::Open && self.pending.is_none() {
            self.state = State::Starting;
            let p = [self.remote << 2];
            self.command(signal::START, &p, out);
        }
    }
    /// Start a stream that is open but suspended.
    pub fn resume(&mut self) -> Vec<Out> { let mut out = Vec::new(); self.resume_into(&mut out); out }
    /// Pause a streaming stream (the headset may sleep its radio).
    pub fn suspend(&mut self) -> Vec<Out> {
        let mut out = Vec::new();
        if self.state == State::Streaming && self.pending.is_none() {
            self.state = State::Suspending;
            let p = [self.remote << 2];
            self.command(signal::SUSPEND, &p, &mut out);
        }
        out
    }
    /// Close the stream.
    pub fn close(&mut self) -> Vec<Out> {
        let mut out = Vec::new();
        if matches!(self.state, State::Open | State::Streaming | State::Configured) {
            self.state = State::Closing;
            let p = [self.remote << 2];
            self.command(signal::CLOSE, &p, &mut out);
        }
        out
    }

    /// One packet from the signaling channel.
    pub fn on_packet(&mut self, p: &[u8]) -> Vec<Out> {
        let mut out = Vec::new();
        if let Some(m) = self.rx.push(p) {
            if m.kind == kind::COMMAND { self.on_command(&m, &mut out); } else { self.on_response(&m, &mut out); }
        }
        out
    }

    fn fail(&mut self, code: u8, out: &mut Vec<Out>) {
        self.state = State::Idle;
        self.config = None;
        self.pending = None;
        out.push(Out::Failed(code));
    }

    fn next_candidate(&mut self, out: &mut Vec<Out>) {
        if self.candidates.is_empty() { return self.fail(error::NO_CONFIGURATION, out); }
        self.remote = self.candidates.remove(0);
        self.state = State::Capabilities;
        let p = [self.remote << 2];
        self.command(signal::GET_CAPABILITIES, &p, out);
    }

    fn on_response(&mut self, m: &Message, out: &mut Vec<Out>) {
        // Only the answer to what we asked, with its label.
        if self.pending != Some((m.label, m.signal)) { return; }
        self.pending = None;
        let ok = m.kind == kind::ACCEPT;
        let code = |m: &Message| *m.params.last().unwrap_or(&error::BAD_HEADER_FORMAT);
        match (m.signal, self.state) {
            (signal::DISCOVER, State::Discovering) => {
                if !ok { return self.fail(code(m), out); }
                self.candidates = parse_seps(&m.params).iter().filter(|s| s.sink && s.media == MEDIA_AUDIO && !s.in_use).map(|s| s.seid).collect();
                if self.candidates.is_empty() { return self.fail(error::NO_SINK, out); }
                self.next_candidate(out);
            }
            (signal::GET_CAPABILITIES, State::Capabilities) => {
                let chosen = if ok { parse_capabilities(&m.params).and_then(|caps| {
                    caps.iter().find(|(c, _)| *c == category::MEDIA_CODEC).and_then(|(_, d)| SbcInfo::from_codec(d)).and_then(|i| i.choose())
                }) } else { None };
                let Some((config, info)) = chosen else { return self.next_candidate(out) };
                self.config = Some(config);
                self.state = State::Configuring;
                let mut p = vec![self.remote << 2, LOCAL_SEID << 2, category::MEDIA_TRANSPORT, 0, category::MEDIA_CODEC, 6];
                p.extend_from_slice(&info.codec());
                self.command(signal::SET_CONFIGURATION, &p, out);
            }
            (signal::SET_CONFIGURATION, State::Configuring) => {
                if !ok { self.config = None; return self.next_candidate(out); }
                self.state = State::Configured;
                out.push(Out::Configured(self.config.unwrap()));
                self.state = State::Opening;
                let p = [self.remote << 2];
                self.command(signal::OPEN, &p, out);
            }
            (signal::OPEN, State::Opening) => {
                if !ok { return self.fail(code(m), out); }
                out.push(Out::OpenMedia);
            }
            (signal::START, State::Starting) => {
                if ok { self.state = State::Streaming; out.push(Out::Streaming); } else { self.state = State::Open; out.push(Out::Failed(code(m))); }
            }
            (signal::SUSPEND, State::Suspending) => {
                // Refused or not, we stop sending; the stream counts as open.
                self.state = State::Open;
                out.push(Out::Suspended);
            }
            (signal::CLOSE, State::Closing) | (signal::ABORT, State::Aborting) => {
                self.state = State::Idle;
                self.config = None;
                out.push(Out::Closed);
            }
            _ => {}
        }
    }

    fn on_command(&mut self, m: &Message, out: &mut Vec<Out>) {
        let (l, s, p) = (m.label, m.signal, m.params.as_slice());
        let ours = |p: &[u8]| p.first().map(|b| b >> 2) == Some(LOCAL_SEID);
        // Rejects: GET_CAPABILITIES style (error only), SET_CONFIGURATION style
        // (category + error), START/SUSPEND style (SEID + error).
        match s {
            signal::DISCOVER => {
                let in_use = !matches!(self.state, State::Idle | State::Discovering | State::Capabilities);
                self.reply(l, kind::ACCEPT, s, &[LOCAL_SEID << 2 | (in_use as u8) << 1, MEDIA_AUDIO << 4], out);
            }
            signal::GET_CAPABILITIES | signal::GET_ALL_CAPABILITIES => {
                if p.is_empty() { return self.reply(l, kind::REJECT, s, &[error::BAD_LENGTH], out); }
                if !ours(p) { return self.reply(l, kind::REJECT, s, &[error::BAD_ACP_SEID], out); }
                let mut r = vec![category::MEDIA_TRANSPORT, 0, category::MEDIA_CODEC, 6];
                r.extend_from_slice(&OUR_SBC.codec());
                self.reply(l, kind::ACCEPT, s, &r, out);
            }
            signal::GET_CONFIGURATION => {
                if p.is_empty() { return self.reply(l, kind::REJECT, s, &[error::BAD_LENGTH], out); }
                if !ours(p) { return self.reply(l, kind::REJECT, s, &[error::BAD_ACP_SEID], out); }
                let Some(c) = self.config else { return self.reply(l, kind::REJECT, s, &[error::BAD_STATE], out) };
                let mut r = vec![category::MEDIA_TRANSPORT, 0, category::MEDIA_CODEC, 6];
                r.extend_from_slice(&info_of(&c).codec());
                self.reply(l, kind::ACCEPT, s, &r, out);
            }
            signal::SET_CONFIGURATION => {
                if p.len() < 2 { return self.reply(l, kind::REJECT, s, &[0, error::BAD_LENGTH], out); }
                if !ours(p) { return self.reply(l, kind::REJECT, s, &[0, error::BAD_ACP_SEID], out); }
                let int = p[1] >> 2;
                if !matches!(self.state, State::Idle | State::Discovering | State::Capabilities) || !(1..=0x3E).contains(&int) {
                    return self.reply(l, kind::REJECT, s, &[0, error::SEP_IN_USE], out);
                }
                let Some(caps) = parse_capabilities(&p[2..]) else { return self.reply(l, kind::REJECT, s, &[0, error::BAD_PAYLOAD_FORMAT], out) };
                let mut transport = false;
                let mut config = None;
                for (cat, d) in &caps {
                    match *cat {
                        category::MEDIA_TRANSPORT if d.is_empty() => transport = true,
                        category::MEDIA_CODEC => match SbcInfo::from_codec(d).and_then(|i| i.config()) {
                            Some(c) => config = Some(c),
                            None => return self.reply(l, kind::REJECT, s, &[category::MEDIA_CODEC, error::UNSUPPORTED_CONFIGURATION], out),
                        },
                        c => return self.reply(l, kind::REJECT, s, &[c, error::BAD_SERV_CATEGORY], out),
                    }
                }
                let (true, Some(c)) = (transport, config) else { return self.reply(l, kind::REJECT, s, &[category::MEDIA_CODEC, error::INVALID_CAPABILITIES], out) };
                // The headset sets the stream up: drop our own attempt.
                self.pending = None;
                self.candidates.clear();
                self.initiator = false;
                self.remote = int;
                self.config = Some(c);
                self.state = State::Configured;
                self.reply(l, kind::ACCEPT, s, &[], out);
                out.push(Out::Configured(c));
            }
            signal::OPEN => {
                if p.is_empty() { return self.reply(l, kind::REJECT, s, &[error::BAD_LENGTH], out); }
                if !ours(p) { return self.reply(l, kind::REJECT, s, &[error::BAD_ACP_SEID], out); }
                if self.state != State::Configured { return self.reply(l, kind::REJECT, s, &[error::BAD_STATE], out); }
                // The headset opens the transport channel next.
                self.state = State::Opening;
                self.reply(l, kind::ACCEPT, s, &[], out);
            }
            signal::START | signal::SUSPEND => {
                if p.is_empty() { return self.reply(l, kind::REJECT, s, &[0, error::BAD_LENGTH], out); }
                if let Some(&bad) = p.iter().find(|b| *b >> 2 != LOCAL_SEID) { return self.reply(l, kind::REJECT, s, &[bad & 0xFC, error::BAD_ACP_SEID], out); }
                let (from, to) = if s == signal::START { (State::Open, State::Streaming) } else { (State::Streaming, State::Open) };
                // Our own START/SUSPEND crossing theirs: theirs wins.
                let crossing = (s == signal::START && self.state == State::Starting) || (s == signal::SUSPEND && self.state == State::Suspending);
                if self.state != from && !crossing { return self.reply(l, kind::REJECT, s, &[LOCAL_SEID << 2, error::BAD_STATE], out); }
                if crossing { self.pending = None; }
                self.state = to;
                self.reply(l, kind::ACCEPT, s, &[], out);
                out.push(if s == signal::START { Out::Streaming } else { Out::Suspended });
            }
            signal::CLOSE => {
                if p.is_empty() { return self.reply(l, kind::REJECT, s, &[error::BAD_LENGTH], out); }
                if !ours(p) { return self.reply(l, kind::REJECT, s, &[error::BAD_ACP_SEID], out); }
                if !matches!(self.state, State::Configured | State::Opening | State::Open | State::Starting | State::Streaming | State::Suspending) {
                    return self.reply(l, kind::REJECT, s, &[error::BAD_STATE], out);
                }
                self.state = State::Idle;
                self.config = None;
                self.pending = None;
                self.reply(l, kind::ACCEPT, s, &[], out);
                out.push(Out::Closed);
            }
            signal::ABORT => {
                // ABORT is never rejected; a malformed one is ignored.
                if !ours(p) { return; }
                self.state = State::Idle;
                self.config = None;
                self.pending = None;
                self.reply(l, kind::ACCEPT, s, &[], out);
                out.push(Out::Closed);
            }
            signal::DELAY_REPORT => self.reply(l, kind::ACCEPT, s, &[], out),
            signal::RECONFIGURE => self.reply(l, kind::REJECT, s, &[0, error::NOT_SUPPORTED_COMMAND], out),
            signal::SECURITY_CONTROL => self.reply(l, kind::REJECT, s, &[error::NOT_SUPPORTED_COMMAND], out),
            _ => self.reply(l, kind::GENERAL_REJECT, s, &[], out),
        }
    }
}

/// The single-choice SBC element of a configuration.
pub fn info_of(c: &sbc::Config) -> SbcInfo {
    let mode = match c.mode { Mode::Mono => 0x08, Mode::DualChannel => 0x04, Mode::Stereo => 0x02, Mode::JointStereo => 0x01 };
    let rate = match c.rate { 16_000 => 0x80, 32_000 => 0x40, 44_100 => 0x20, _ => 0x10 };
    let blocks = match c.blocks { 4 => 0x80, 8 => 0x40, 12 => 0x20, _ => 0x10 };
    let sub = if c.subbands == 4 { 0x08 } else { 0x04 };
    let alloc = if c.allocation == Allocation::Snr { 0x02 } else { 0x01 };
    SbcInfo([rate | mode, blocks | sub | alloc, 2, c.bitpool])
}
