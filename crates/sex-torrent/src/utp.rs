// SPDX-License-Identifier: GPL-3.0-only
//! uTP, the Micro Transport Protocol (BEP 29): reliable, ordered byte streams over UDP
//! whose congestion control (LEDBAT, see [`crate::ledbat`]) yields to other traffic, so
//! a busy torrent does not make the rest of the household's internet slow.
//!
//! Sans-IO. [`UtpSocket`] stands for one UDP socket: hand it every datagram that arrived
//! ([`UtpSocket::on_datagram`]), send what [`UtpSocket::poll_transmit`] returns, and call
//! `poll_transmit` again at [`UtpSocket::next_timeout`]. It routes datagrams by
//! (address, connection id) to its connections ([`UtpConn`]), opens outgoing ones
//! ([`UtpSocket::connect`]) and accepts incoming SYNs into a bounded backlog
//! ([`UtpSocket::accept`]). Time is the caller's microsecond clock (`now_us`).
//!
//! Wire format: a 20-byte header (type and version, first extension, connection id,
//! send timestamp, timestamp difference, receive window, seq_nr, ack_nr), a chain of
//! extensions (we use 1, selective ACK: a bitmask of received packets past ack_nr + 1),
//! then the payload. The connecting side picks a random receive id R, sends with R + 1
//! and puts R in its SYN; the accepting side receives on R + 1 and sends with R.
//!
//! Reliability as in BEP 29 and libutp: every data packet consumes a sequence number,
//! STATE packets (pure acks) do not. Round-trip time is smoothed as in TCP (RFC 6298)
//! and the retransmission timeout is `max(rtt + 4 * rtt_var, 500 ms)`, doubling on each
//! timeout. Three duplicate acks, or three selectively acked packets past a hole, mark
//! the hole lost and resend it at once. FIN closes a direction gracefully, RESET aborts.
//! Three unanswered SYNs (about 7 s) are a connect timeout; an idle connection sends a
//! keepalive every 29 s and is dropped after 90 s of silence.

use crate::ledbat::Ledbat;
use crate::PeerAddr;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::vec::Vec;

/// The protocol version in the header.
pub const VERSION: u8 = 1;
/// Fixed header size.
pub const HEADER_LEN: usize = 20;
/// Default largest datagram we send (header included); fits common tunnels and PPPoE.
pub const DEFAULT_MTU: usize = 1400;
/// Retransmission timeout bounds and start value (microseconds).
pub const MIN_RTO_US: u64 = 500_000;
pub const MAX_RTO_US: u64 = 60_000_000;
pub const INITIAL_RTO_US: u64 = 1_000_000;
/// SYN attempts before giving up (1 s, 2 s, 4 s apart).
pub const MAX_SYN_ATTEMPTS: u32 = 3;
/// Consecutive timeouts on data before the connection is declared dead.
pub const MAX_TIMEOUTS: u32 = 7;
/// Keepalive interval and idle limit (microseconds).
pub const KEEPALIVE_US: u64 = 29_000_000;
pub const IDLE_TIMEOUT_US: u64 = 90_000_000;
/// How long a gracefully closed connection lingers to re-acknowledge a lost final ack
/// (at least this, at most the maximum, otherwise twice the retransmission timeout;
/// counted from the last packet received).
pub const TIME_WAIT_US: u64 = 15_000_000;
pub const MAX_TIME_WAIT_US: u64 = 60_000_000;
/// Delay before probing a peer that advertised a zero receive window.
pub const ZERO_WINDOW_PROBE_US: u64 = 1_000_000;
/// Out-of-order packets kept per connection (also the reach of our selective acks).
pub const REORDER_SLOTS: usize = 257;
/// Duplicate acks (or selectively acked packets past a hole) that mean "lost".
pub const DUP_ACK_THRESHOLD: u32 = 3;

/// The five packet types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PacketType { Data = 0, Fin = 1, State = 2, Reset = 3, Syn = 4 }

impl PacketType {
    fn from_u8(v: u8) -> Option<Self> {
        Some(match v { 0 => Self::Data, 1 => Self::Fin, 2 => Self::State, 3 => Self::Reset, 4 => Self::Syn, _ => return None })
    }
}

/// The fixed header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub ty: PacketType,
    pub conn_id: u16,
    /// Sender's clock when sent, microseconds (wrapping).
    pub timestamp_us: u32,
    /// Sender's last measured one-way delay of our packets (its clock minus ours).
    pub timestamp_diff_us: u32,
    /// Sender's free receive buffer in bytes.
    pub wnd_size: u32,
    pub seq_nr: u16,
    pub ack_nr: u16,
}

/// A parsed datagram, borrowing from the input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Packet<'a> {
    pub header: Header,
    /// Selective ACK bitmask (extension 1): bit i of byte i/8 (LSB first) set means
    /// seq ack_nr + 2 + 8 * (i/8) + (i%8) arrived.
    pub sack: Option<&'a [u8]>,
    pub payload: &'a [u8],
}

fn be16(b: &[u8], at: usize) -> Option<u16> { Some(u16::from_be_bytes([*b.get(at)?, *b.get(at + 1)?])) }
fn be32(b: &[u8], at: usize) -> Option<u32> { Some(u32::from_be_bytes([*b.get(at)?, *b.get(at + 1)?, *b.get(at + 2)?, *b.get(at + 3)?])) }

/// True if the bytes look like a uTP packet (for telling uTP from DHT on a shared port).
pub fn looks_like_utp(b: &[u8]) -> bool {
    b.len() >= HEADER_LEN && b[0] & 0x0f == VERSION && b[0] >> 4 <= 4 && b[1] <= 4
}

/// Parses a datagram; `None` for anything malformed (never panics).
pub fn parse(b: &[u8]) -> Option<Packet<'_>> {
    let first = *b.first()?;
    if first & 0x0f != VERSION || b.len() < HEADER_LEN { return None; }
    let header = Header {
        ty: PacketType::from_u8(first >> 4)?,
        conn_id: be16(b, 2)?,
        timestamp_us: be32(b, 4)?,
        timestamp_diff_us: be32(b, 8)?,
        wnd_size: be32(b, 12)?,
        seq_nr: be16(b, 16)?,
        ack_nr: be16(b, 18)?,
    };
    let mut next = *b.get(1)?;
    let mut pos = HEADER_LEN;
    let mut sack = None;
    // Each extension takes at least two bytes, so this ends.
    while next != 0 {
        let following = *b.get(pos)?;
        let len = *b.get(pos + 1)? as usize;
        let data = b.get(pos + 2..pos + 2 + len)?;
        if next == 1 {
            if len == 0 || len % 4 != 0 { return None; }
            sack = Some(data);
        }
        pos += 2 + len;
        next = following;
    }
    Some(Packet { header, sack, payload: b.get(pos..)? })
}

/// Encodes a packet.
pub fn encode(h: &Header, sack: Option<&[u8]>, payload: &[u8]) -> Vec<u8> {
    let sack = sack.filter(|s| !s.is_empty() && s.len() % 4 == 0 && s.len() <= 252);
    let mut v = Vec::with_capacity(HEADER_LEN + sack.map_or(0, |s| s.len() + 2) + payload.len());
    v.push(((h.ty as u8) << 4) | VERSION);
    v.push(if sack.is_some() { 1 } else { 0 });
    v.extend_from_slice(&h.conn_id.to_be_bytes());
    v.extend_from_slice(&h.timestamp_us.to_be_bytes());
    v.extend_from_slice(&h.timestamp_diff_us.to_be_bytes());
    v.extend_from_slice(&h.wnd_size.to_be_bytes());
    v.extend_from_slice(&h.seq_nr.to_be_bytes());
    v.extend_from_slice(&h.ack_nr.to_be_bytes());
    if let Some(s) = sack {
        v.push(0);
        v.push(s.len() as u8);
        v.extend_from_slice(s);
    }
    v.extend_from_slice(payload);
    v
}

/// Signed distance a - b on the 16-bit sequence space.
fn seq_diff(a: u16, b: u16) -> i16 { a.wrapping_sub(b) as i16 }

/// Socket and connection settings.
#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// Largest datagram sent, header included.
    pub mtu: usize,
    /// Bytes `write` may buffer, unacknowledged data included.
    pub send_buffer: usize,
    /// Bytes we buffer for the reader (the advertised receive window).
    pub recv_buffer: usize,
    /// Most connections the socket holds.
    pub max_conns: usize,
    /// Most accepted connections waiting for [`UtpSocket::accept`].
    pub backlog: usize,
}

impl Default for Config {
    fn default() -> Self { Config { mtu: DEFAULT_MTU, send_buffer: 1 << 20, recv_buffer: 1 << 20, max_conns: 2048, backlog: 64 } }
}

/// Where a connection is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State { SynSent, Connected, Closed }

/// Why a connection closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseReason {
    /// Both sides finished: FINs exchanged and acknowledged.
    Graceful,
    /// The peer sent RESET.
    Reset,
    /// No answer to our SYNs.
    ConnectTimeout,
    /// Data went unacknowledged too long, or the peer went silent.
    Timeout,
    /// We aborted it ([`UtpConn::abort`]).
    Aborted,
}

#[derive(Clone, Debug)]
struct Sent {
    seq: u16,
    ty: PacketType,
    payload: Vec<u8>,
    sent_us: u64,
    transmissions: u32,
    need_resend: bool,
    /// Counted in `flight` (bytes the window is charged for).
    counted: bool,
    sacked: bool,
    fast_resent: bool,
}

/// Something that happened on a connection, reported through [`UtpSocket::poll_event`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// Our SYN was answered: writes go out now.
    Connected(ConnKey),
    /// Data (or the end of the stream) is ready to read.
    Readable(ConnKey),
    /// A `write` that was cut short can be retried.
    Writable(ConnKey),
    /// The connection is closed (it is removed once its data was read).
    Closed(ConnKey, CloseReason),
}

/// Names a connection on a socket: the peer and our receive id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConnKey { pub addr: PeerAddr, pub recv_id: u16 }

/// One uTP connection.
#[derive(Debug)]
pub struct UtpConn {
    key: ConnKey,
    send_id: u16,
    state: State,
    reason: Option<CloseReason>,
    cfg: Config,
    // Send side.
    seq_nr: u16,
    send_buf: VecDeque<u8>,
    inflight: VecDeque<Sent>,
    flight: u32,
    unacked_bytes: usize,
    peer_wnd: u32,
    zero_wnd_since: Option<u64>,
    cc: Ledbat,
    srtt: Option<u64>,
    rttvar: u64,
    rto: u64,
    rto_deadline: Option<u64>,
    timeouts: u32,
    syn_attempts: u32,
    dup_acks: u32,
    last_ack: u16,
    closing: bool,
    fin_sent: bool,
    fin_acked_at: Option<u64>,
    write_blocked: bool,
    reset_pending: bool,
    closed_at: Option<u64>,
    now_us: u64,
    /// Responder only, until the initiator is heard from: our first seq_nr, which every
    /// STATE carries meanwhile (the initiator derives its ack_nr from it, even if we
    /// already sent data and the first reply was lost).
    syn_seq: Option<u16>,
    // Receive side.
    ack_nr: u16,
    rx: VecDeque<u8>,
    reorder: VecDeque<Option<(PacketType, Vec<u8>)>>,
    reorder_bytes: usize,
    eof_seq: Option<u16>,
    eof: bool,
    reply_micro: u32,
    ack_pending: bool,
    last_adv_wnd: u32,
    last_send_us: u64,
    last_recv_us: u64,
    events: Vec<Event>,
}

impl UtpConn {
    fn new(key: ConnKey, send_id: u16, seq_nr: u16, cfg: Config, now_us: u64) -> Self {
        let mss = (cfg.mtu.max(HEADER_LEN + 64) - HEADER_LEN) as u32;
        UtpConn {
            key, send_id, state: State::SynSent, reason: None, cfg,
            seq_nr, send_buf: VecDeque::new(), inflight: VecDeque::new(), flight: 0, unacked_bytes: 0,
            peer_wnd: cfg.recv_buffer as u32, zero_wnd_since: None,
            cc: Ledbat::new(mss, cfg.send_buffer.max(1) as u32, now_us),
            srtt: None, rttvar: 0, rto: INITIAL_RTO_US, rto_deadline: None, timeouts: 0, syn_attempts: 0,
            dup_acks: 0, last_ack: seq_nr.wrapping_sub(1),
            closing: false, fin_sent: false, fin_acked_at: None, write_blocked: false, reset_pending: false, closed_at: None, now_us, syn_seq: None,
            ack_nr: 0, rx: VecDeque::new(), reorder: VecDeque::new(), reorder_bytes: 0, eof_seq: None, eof: false,
            reply_micro: 0, ack_pending: false, last_adv_wnd: cfg.recv_buffer as u32,
            last_send_us: now_us, last_recv_us: now_us, events: Vec::new(),
        }
    }

    fn mss(&self) -> usize { self.cfg.mtu.max(HEADER_LEN + 64) - HEADER_LEN }

    pub fn key(&self) -> ConnKey { self.key }
    pub fn state(&self) -> State { self.state }
    pub fn close_reason(&self) -> Option<CloseReason> { self.reason }
    /// The smoothed round-trip time, once measured.
    pub fn rtt_us(&self) -> Option<u64> { self.srtt }
    /// The congestion window in bytes.
    pub fn cwnd(&self) -> u32 { self.cc.window() }
    /// Bytes waiting to be sent or acknowledged.
    pub fn send_queue(&self) -> usize { self.send_buf.len() + self.unacked_bytes }
    /// Bytes ready for `read`.
    pub fn readable(&self) -> usize { self.rx.len() }
    /// The peer finished sending and everything was read.
    pub fn is_eof(&self) -> bool { self.eof && self.rx.is_empty() }

    /// Queues bytes to send; returns how many fit in the send buffer.
    pub fn write(&mut self, data: &[u8]) -> usize {
        if self.closing || self.state == State::Closed { return 0; }
        let room = self.cfg.send_buffer.saturating_sub(self.send_queue());
        let n = room.min(data.len());
        self.send_buf.extend(&data[..n]);
        if n < data.len() { self.write_blocked = true; }
        n
    }

    /// Reads received bytes in order; 0 when none are ready (see [`UtpConn::is_eof`]).
    pub fn read(&mut self, buf: &mut [u8]) -> usize {
        let n = buf.len().min(self.rx.len());
        for (d, s) in buf.iter_mut().zip(self.rx.drain(..n)) { *d = s; }
        // Tell the peer when our window reopens.
        let wnd = self.recv_window();
        if n > 0 && self.last_adv_wnd < (self.cfg.recv_buffer / 2) as u32 && wnd >= (self.cfg.recv_buffer / 2) as u32 {
            self.ack_pending = true;
        }
        n
    }

    /// Graceful close: sends FIN once the buffered data has gone out. Reading still works
    /// until the peer's FIN.
    pub fn close(&mut self) { self.closing = true; }

    /// Aborts: sends RESET and closes at once.
    pub fn abort(&mut self) {
        if self.state != State::Closed { self.closing = true; self.set_closed(CloseReason::Aborted); self.ack_pending = false; self.reset_pending = true; }
    }

    fn recv_window(&self) -> u32 {
        self.cfg.recv_buffer.saturating_sub(self.rx.len() + self.reorder_bytes) as u32
    }

    fn set_closed(&mut self, r: CloseReason) {
        if self.state != State::Closed {
            self.state = State::Closed;
            self.reason = Some(r);
            self.closed_at = Some(self.now_us);
            self.rto_deadline = None;
            self.events.push(Event::Closed(self.key, r));
        }
    }

    fn header(&mut self, ty: PacketType, seq: u16, now_us: u64) -> Header {
        let wnd = self.recv_window();
        self.last_adv_wnd = wnd;
        Header {
            ty, conn_id: if ty == PacketType::Syn { self.key.recv_id } else { self.send_id },
            timestamp_us: now_us as u32, timestamp_diff_us: self.reply_micro, wnd_size: wnd, seq_nr: seq, ack_nr: self.ack_nr,
        }
    }

    /// The selective ACK bitmask for what sits in the reorder buffer.
    fn sack_mask(&self) -> Option<Vec<u8>> {
        if !self.reorder.iter().any(|s| s.is_some()) { return None; }
        let bits = self.reorder.len().saturating_sub(1);
        let bytes = (bits.div_ceil(8).max(1) + 3) / 4 * 4;
        let mut m = alloc::vec![0u8; bytes];
        for (i, slot) in self.reorder.iter().enumerate().skip(1) {
            if slot.is_some() { if let Some(b) = m.get_mut((i - 1) / 8) { *b |= 1 << ((i - 1) % 8); } }
        }
        Some(m)
    }

    fn update_rtt(&mut self, sample: u64) {
        match self.srtt {
            None => { self.srtt = Some(sample); self.rttvar = sample / 2; }
            Some(s) => {
                let delta = s.abs_diff(sample);
                self.rttvar = (3 * self.rttvar + delta) / 4;
                self.srtt = Some((7 * s + sample) / 8);
            }
        }
        self.rto = (self.srtt.unwrap_or(sample) + 4 * self.rttvar).clamp(MIN_RTO_US, MAX_RTO_US);
    }

    /// Handles one packet addressed to this connection.
    fn on_packet(&mut self, p: &Packet<'_>, now_us: u64) {
        let h = p.header;
        self.now_us = self.now_us.max(now_us);
        if self.state == State::Closed {
            if h.ty != PacketType::Fin && h.ty != PacketType::Data { return; }
            if self.reason == Some(CloseReason::Graceful) { self.closed_at = Some(now_us); }
        }
        self.last_recv_us = now_us;
        if h.timestamp_us != 0 { self.reply_micro = (now_us as u32).wrapping_sub(h.timestamp_us); }
        match h.ty {
            PacketType::Reset => {
                // After both FINs, with all our data acknowledged, a RESET only means the
                // peer already forgot the connection (its final ack was lost).
                let done = self.eof && self.fin_sent && self.inflight.iter().all(|s| s.ty == PacketType::Fin);
                self.set_closed(if done { CloseReason::Graceful } else { CloseReason::Reset });
                return;
            }
            PacketType::Syn => { self.ack_pending = true; return; } // duplicate SYN: ack again
            _ => self.syn_seq = None, // the initiator got our SYN reply
        }
        if self.state == State::SynSent {
            if h.ty != PacketType::State || Some(h.ack_nr) != self.inflight.front().map(|s| s.seq) { return; }
            self.ack_nr = h.seq_nr.wrapping_sub(1);
            self.state = State::Connected;
            self.events.push(Event::Connected(self.key));
        }
        self.peer_wnd = h.wnd_size;
        if self.peer_wnd as usize >= self.mss() { self.zero_wnd_since = None; } else if self.zero_wnd_since.is_none() { self.zero_wnd_since = Some(now_us); }
        self.process_ack(&h, p.sack, now_us);
        if matches!(h.ty, PacketType::Data | PacketType::Fin) { self.process_data(h.ty, h.seq_nr, p.payload); }
        if self.eof && self.fin_acked_at.is_some() { self.set_closed(CloseReason::Graceful); }
    }

    fn process_ack(&mut self, h: &Header, sack: Option<&[u8]>, now_us: u64) {
        let flight_before = self.flight;
        let mut acked_bytes = 0u32;
        let base = self.seq_nr.wrapping_sub(self.inflight.len() as u16).wrapping_sub(1); // last acked
        let d = seq_diff(h.ack_nr, base);
        if d < 0 || d as usize > self.inflight.len() { return; } // acks something never sent, or very old
        let mut progressed = false;
        while let Some(front) = self.inflight.front() {
            if seq_diff(h.ack_nr, front.seq) < 0 { break; }
            let Some(s) = self.inflight.pop_front() else { break };
            progressed = true;
            if s.counted { self.flight = self.flight.saturating_sub(s.payload.len() as u32); }
            self.unacked_bytes = self.unacked_bytes.saturating_sub(s.payload.len());
            if !s.sacked {
                acked_bytes += s.payload.len() as u32;
                if s.transmissions == 1 { self.update_rtt(now_us.saturating_sub(s.sent_us)); }
            }
            if s.ty == PacketType::Fin { self.fin_acked_at = Some(now_us); }
        }
        // Selective acks.
        let mut lost = false;
        if let Some(mask) = sack {
            let first = h.ack_nr.wrapping_add(2);
            for s in self.inflight.iter_mut() {
                let i = seq_diff(s.seq, first);
                if i < 0 || i as usize >= mask.len() * 8 { continue; }
                let i = i as usize;
                if mask.get(i / 8).is_some_and(|b| b & (1 << (i % 8)) != 0) && !s.sacked {
                    s.sacked = true;
                    s.need_resend = false;
                    if s.counted { s.counted = false; self.flight = self.flight.saturating_sub(s.payload.len() as u32); }
                    acked_bytes += s.payload.len() as u32;
                    if s.transmissions == 1 {
                        let sample = now_us.saturating_sub(s.sent_us);
                        self.srtt = Some(self.srtt.map_or(sample, |r| (7 * r + sample) / 8));
                    }
                }
            }
            // A hole with DUP_ACK_THRESHOLD sacked packets after it is lost.
            let mut sacked_after = 0u32;
            for s in self.inflight.iter_mut().rev() {
                if s.sacked { sacked_after += 1; continue; }
                if sacked_after >= DUP_ACK_THRESHOLD && !s.fast_resent && s.ty != PacketType::Syn {
                    s.fast_resent = true;
                    s.need_resend = true;
                    lost = true;
                }
            }
        }
        if progressed {
            self.dup_acks = 0;
            self.timeouts = 0;
            self.last_ack = h.ack_nr;
            self.rto_deadline = if self.inflight.iter().any(|s| !s.sacked) { Some(now_us + self.rto) } else { None };
            if self.write_blocked && self.send_queue() < self.cfg.send_buffer {
                self.write_blocked = false;
                self.events.push(Event::Writable(self.key));
            }
        } else if h.ty == PacketType::State && h.ack_nr == self.last_ack && !self.inflight.is_empty() {
            self.dup_acks += 1;
            if self.dup_acks == DUP_ACK_THRESHOLD {
                if let Some(f) = self.inflight.front_mut() {
                    if !f.fast_resent && !f.sacked { f.fast_resent = true; f.need_resend = true; lost = true; }
                }
            }
        }
        if lost { self.cc.on_loss(self.srtt.unwrap_or(self.rto), now_us); }
        if acked_bytes > 0 { self.cc.on_ack(acked_bytes, h.timestamp_diff_us, flight_before, now_us); }
    }

    fn deliver(&mut self, ty: PacketType, payload: &[u8]) {
        if ty == PacketType::Fin { self.eof = true; return; }
        if !payload.is_empty() {
            let was_empty = self.rx.is_empty();
            self.rx.extend(payload);
            if was_empty { self.events.push(Event::Readable(self.key)); }
        }
    }

    fn process_data(&mut self, ty: PacketType, seq: u16, payload: &[u8]) {
        self.ack_pending = true;
        if self.eof { return; }
        if let Some(e) = self.eof_seq { if seq_diff(seq, e) > 0 { return; } }
        if ty == PacketType::Fin { self.eof_seq = Some(seq); }
        let d = seq_diff(seq, self.ack_nr.wrapping_add(1));
        if d < 0 || d as usize >= REORDER_SLOTS { return; } // old duplicate, or too far ahead
        let d = d as usize;
        // Bound memory by our advertised window.
        if self.rx.len() + self.reorder_bytes + payload.len() > self.cfg.recv_buffer { return; }
        if d == 0 {
            self.deliver(ty, payload);
            self.ack_nr = seq;
            if !self.reorder.is_empty() { self.reorder.pop_front(); }
            while let Some(Some(_)) = self.reorder.front() {
                if let Some(Some((t, p))) = self.reorder.pop_front() {
                    self.reorder_bytes = self.reorder_bytes.saturating_sub(p.len());
                    self.deliver(t, &p);
                    self.ack_nr = self.ack_nr.wrapping_add(1);
                }
            }
            if self.eof && self.rx.is_empty() { self.events.push(Event::Readable(self.key)); }
        } else {
            if self.reorder.len() <= d { self.reorder.resize(d + 1, None); }
            if let Some(slot) = self.reorder.get_mut(d) {
                if slot.is_none() { self.reorder_bytes += payload.len(); *slot = Some((ty, payload.to_vec())); }
            }
        }
    }

    /// Runs the timers.
    fn on_timer(&mut self, now_us: u64) {
        self.now_us = self.now_us.max(now_us);
        if self.state == State::Closed { return; }
        if let Some(t) = self.rto_deadline {
            if now_us >= t {
                if self.state == State::SynSent {
                    self.syn_attempts += 1;
                    if self.syn_attempts >= MAX_SYN_ATTEMPTS { self.set_closed(CloseReason::ConnectTimeout); return; }
                } else if self.peer_wnd as usize >= self.mss() {
                    self.timeouts += 1;
                    if self.timeouts >= MAX_TIMEOUTS { self.set_closed(CloseReason::Timeout); self.reset_pending = true; return; }
                }
                self.rto = (self.rto * 2).min(MAX_RTO_US);
                self.cc.on_timeout(now_us);
                for s in self.inflight.iter_mut().filter(|s| !s.sacked) {
                    s.need_resend = true;
                    if s.counted { s.counted = false; self.flight = self.flight.saturating_sub(s.payload.len() as u32); }
                }
                self.rto_deadline = Some(now_us + self.rto);
            }
        }
        if now_us.saturating_sub(self.last_recv_us) >= IDLE_TIMEOUT_US { self.set_closed(CloseReason::Timeout); return; }
        if self.state == State::Connected && now_us.saturating_sub(self.last_send_us) >= KEEPALIVE_US { self.ack_pending = true; }
    }

    fn next_deadline(&self) -> Option<u64> {
        if self.state == State::Closed {
            return self.closed_at.map(|t| t + (2 * self.rto).clamp(TIME_WAIT_US, MAX_TIME_WAIT_US));
        }
        let mut t = self.last_recv_us + IDLE_TIMEOUT_US;
        if let Some(r) = self.rto_deadline { t = t.min(r); }
        if self.state == State::Connected { t = t.min(self.last_send_us + KEEPALIVE_US); }
        if let Some(z) = self.zero_wnd_since { if self.flight == 0 && !self.send_buf.is_empty() { t = t.min(z + ZERO_WINDOW_PROBE_US); } }
        Some(t)
    }

    fn window_allows(&self, len: usize, now_us: u64) -> bool {
        let wnd = self.cc.window().min(self.peer_wnd) as usize;
        if self.flight as usize + len <= wnd { return true; }
        // Always let one packet out when nothing is in flight (a zero window is probed later).
        self.flight == 0 && (self.peer_wnd as usize >= self.mss() || self.zero_wnd_since.is_some_and(|z| now_us >= z + ZERO_WINDOW_PROBE_US))
    }

    /// The next packet this connection wants to send.
    fn poll_packet(&mut self, now_us: u64) -> Option<Vec<u8>> {
        if self.reset_pending {
            self.reset_pending = false;
            let h = self.header(PacketType::Reset, self.seq_nr, now_us);
            return Some(encode(&h, None, &[]));
        }
        if self.state == State::Closed {
            if !self.ack_pending { return None; }
            self.ack_pending = false;
            let h = self.header(PacketType::State, self.seq_nr, now_us);
            return Some(encode(&h, None, &[]));
        }
        // Retransmissions first.
        let idx = self.inflight.iter().position(|s| s.need_resend && !s.sacked);
        if let Some(i) = idx {
            let (len, counted) = self.inflight.get(i).map(|s| (s.payload.len(), s.counted))?;
            if counted || self.window_allows(len, now_us) {
                let (ty, seq, payload) = {
                    let s = self.inflight.get_mut(i)?;
                    s.need_resend = false;
                    s.transmissions += 1;
                    s.sent_us = now_us;
                    if !s.counted { s.counted = true; }
                    (s.ty, s.seq, s.payload.clone())
                };
                if !counted { self.flight += len as u32; }
                return Some(self.emit(ty, seq, &payload, now_us));
            }
        }
        if self.state == State::SynSent { return None; }
        // New data.
        if !self.send_buf.is_empty() {
            let len = self.send_buf.len().min(self.mss());
            if self.window_allows(len, now_us) && self.inflight.len() < 0x7fff {
                let payload: Vec<u8> = self.send_buf.drain(..len).collect();
                return Some(self.push_new(PacketType::Data, payload, now_us));
            }
        }
        // FIN once everything was handed to the network.
        if self.closing && !self.fin_sent && self.send_buf.is_empty() {
            self.fin_sent = true;
            return Some(self.push_new(PacketType::Fin, Vec::new(), now_us));
        }
        if self.ack_pending {
            let h = self.header(PacketType::State, self.syn_seq.unwrap_or(self.seq_nr), now_us);
            self.ack_pending = false;
            self.last_send_us = now_us;
            return Some(encode(&h, self.sack_mask().as_deref(), &[]));
        }
        None
    }

    fn push_new(&mut self, ty: PacketType, payload: Vec<u8>, now_us: u64) -> Vec<u8> {
        let seq = self.seq_nr;
        self.seq_nr = self.seq_nr.wrapping_add(1);
        let out = self.emit(ty, seq, &payload, now_us);
        self.flight += payload.len() as u32;
        self.unacked_bytes += payload.len();
        self.inflight.push_back(Sent { seq, ty, payload, sent_us: now_us, transmissions: 1, need_resend: false, counted: true, sacked: false, fast_resent: false });
        if self.rto_deadline.is_none() { self.rto_deadline = Some(now_us + self.rto); }
        out
    }

    fn emit(&mut self, ty: PacketType, seq: u16, payload: &[u8], now_us: u64) -> Vec<u8> {
        let h = self.header(ty, seq, now_us);
        // A data packet carries the ack too; the bitmask only rides on STATE packets.
        if self.reorder.iter().all(|s| s.is_none()) { self.ack_pending = false; }
        self.last_send_us = now_us;
        if self.rto_deadline.is_none() { self.rto_deadline = Some(now_us + self.rto); }
        encode(&h, None, payload)
    }

    /// Closed, everything sent and read, and (after a graceful close) the TIME_WAIT
    /// period over, during which retransmitted FINs are still acknowledged instead of
    /// being answered with RESET.
    fn removable(&self, now_us: u64) -> bool {
        let waited = self.reason != Some(CloseReason::Graceful)
            || self.closed_at.is_some_and(|t| now_us >= t + (2 * self.rto).clamp(TIME_WAIT_US, MAX_TIME_WAIT_US));
        self.state == State::Closed && !self.ack_pending && !self.reset_pending && self.rx.is_empty() && waited
    }
}

/// A splitmix64 generator for connection ids and sequence numbers (not secret).
#[derive(Clone, Debug)]
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// Errors from [`UtpSocket::connect`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectError { TooManyConnections }

/// All uTP connections on one UDP socket.
#[derive(Debug)]
pub struct UtpSocket {
    cfg: Config,
    conns: BTreeMap<ConnKey, UtpConn>,
    backlog: VecDeque<ConnKey>,
    resets: VecDeque<(PeerAddr, Vec<u8>)>,
    events: VecDeque<Event>,
    cursor: Option<ConnKey>,
    rng: Rng,
}

const MAX_QUEUED_RESETS: usize = 64;
const MAX_QUEUED_EVENTS: usize = 65_536;

impl UtpSocket {
    /// `seed` should come from the program's random source; it picks connection ids.
    pub fn new(cfg: Config, seed: u64) -> Self {
        UtpSocket { cfg, conns: BTreeMap::new(), backlog: VecDeque::new(), resets: VecDeque::new(), events: VecDeque::new(), cursor: None, rng: Rng(seed) }
    }

    /// Opens a connection; the SYN goes out on the next `poll_transmit`.
    pub fn connect(&mut self, addr: PeerAddr, now_us: u64) -> Result<ConnKey, ConnectError> {
        if self.conns.len() >= self.cfg.max_conns { return Err(ConnectError::TooManyConnections); }
        let key = loop {
            let id = self.rng.next() as u16;
            let k = ConnKey { addr, recv_id: id };
            // Our send id (id + 1) must not look like another connection's receive id.
            if !self.conns.contains_key(&k) && !self.conns.contains_key(&ConnKey { addr, recv_id: id.wrapping_add(1) }) { break k; }
        };
        let seq = self.rng.next() as u16;
        let mut c = UtpConn::new(key, key.recv_id.wrapping_add(1), seq, self.cfg, now_us);
        c.seq_nr = seq.wrapping_add(1);
        c.inflight.push_back(Sent { seq, ty: PacketType::Syn, payload: Vec::new(), sent_us: now_us, transmissions: 0, need_resend: true, counted: true, sacked: false, fast_resent: false });
        c.rto_deadline = Some(now_us + c.rto);
        self.conns.insert(key, c);
        Ok(key)
    }

    /// The next accepted incoming connection.
    pub fn accept(&mut self) -> Option<ConnKey> {
        while let Some(k) = self.backlog.pop_front() { if self.conns.contains_key(&k) { return Some(k); } }
        None
    }

    pub fn conn(&self, key: &ConnKey) -> Option<&UtpConn> { self.conns.get(key) }
    pub fn conn_mut(&mut self, key: &ConnKey) -> Option<&mut UtpConn> { self.conns.get_mut(key) }
    pub fn len(&self) -> usize { self.conns.len() }
    pub fn is_empty(&self) -> bool { self.conns.is_empty() }

    /// Writes to a connection; 0 if it does not exist.
    pub fn write(&mut self, key: &ConnKey, data: &[u8]) -> usize { self.conns.get_mut(key).map_or(0, |c| c.write(data)) }
    /// Reads from a connection; 0 if it does not exist.
    pub fn read(&mut self, key: &ConnKey, buf: &mut [u8]) -> usize { self.conns.get_mut(key).map_or(0, |c| c.read(buf)) }

    /// The next event.
    pub fn poll_event(&mut self) -> Option<Event> { self.events.pop_front() }

    fn collect(&mut self, key: &ConnKey) {
        if let Some(c) = self.conns.get_mut(key) {
            for e in c.events.drain(..) { if self.events.len() < MAX_QUEUED_EVENTS { self.events.push_back(e); } }
        }
    }

    fn queue_reset(&mut self, to: PeerAddr, conn_id: u16, ack_nr: u16, now_us: u64) {
        if self.resets.len() >= MAX_QUEUED_RESETS { return; }
        let h = Header { ty: PacketType::Reset, conn_id, timestamp_us: now_us as u32, timestamp_diff_us: 0, wnd_size: 0, seq_nr: self.rng.next() as u16, ack_nr };
        self.resets.push_back((to, encode(&h, None, &[])));
    }

    /// Handles a datagram from `from`. Returns false if it was not uTP (e.g. DHT).
    pub fn on_datagram(&mut self, from: PeerAddr, bytes: &[u8], now_us: u64) -> bool {
        let Some(p) = parse(bytes) else { return false };
        let h = p.header;
        let key = ConnKey { addr: from, recv_id: h.conn_id };
        match h.ty {
            PacketType::Syn => {
                let k = ConnKey { addr: from, recv_id: h.conn_id.wrapping_add(1) };
                if let Some(c) = self.conns.get_mut(&k) {
                    c.on_packet(&p, now_us);
                } else if self.conns.len() >= self.cfg.max_conns || self.backlog.len() >= self.cfg.backlog {
                    self.queue_reset(from, h.conn_id, h.seq_nr, now_us);
                } else {
                    let seq = self.rng.next() as u16;
                    let mut c = UtpConn::new(k, h.conn_id, seq, self.cfg, now_us);
                    c.state = State::Connected;
                    c.syn_seq = Some(seq);
                    c.ack_nr = h.seq_nr;
                    c.peer_wnd = h.wnd_size;
                    if h.timestamp_us != 0 { c.reply_micro = (now_us as u32).wrapping_sub(h.timestamp_us); }
                    c.ack_pending = true;
                    self.conns.insert(k, c);
                    self.backlog.push_back(k);
                }
            }
            PacketType::Reset => {
                // A RESET may carry either of the connection's ids.
                let cands = [key, ConnKey { addr: from, recv_id: h.conn_id.wrapping_sub(1) }, ConnKey { addr: from, recv_id: h.conn_id.wrapping_add(1) }];
                for (i, k) in cands.iter().enumerate() {
                    if let Some(c) = self.conns.get_mut(k) {
                        if i == 0 || c.send_id == h.conn_id { c.on_packet(&p, now_us); self.collect(k); break; }
                    }
                }
            }
            _ => {
                if let Some(c) = self.conns.get_mut(&key) {
                    c.on_packet(&p, now_us);
                    self.collect(&key);
                } else {
                    self.queue_reset(from, h.conn_id, h.seq_nr, now_us);
                }
            }
        }
        true
    }

    /// The next datagram to send. Also runs timers, so call it until it returns `None`
    /// after every input and at [`UtpSocket::next_timeout`].
    pub fn poll_transmit(&mut self, now_us: u64) -> Option<(PeerAddr, Vec<u8>)> {
        if let Some(r) = self.resets.pop_front() { return Some(r); }
        // Round-robin from the cursor so one busy connection cannot starve the others.
        let keys: Vec<ConnKey> = match self.cursor {
            Some(c) => {
                use core::ops::Bound::{Excluded, Unbounded};
                self.conns.range((Excluded(c), Unbounded)).chain(self.conns.range(..=c)).map(|(k, _)| *k).collect()
            }
            None => self.conns.keys().copied().collect(),
        };
        let mut dead = Vec::new();
        let mut result = None;
        for k in keys {
            let Some(c) = self.conns.get_mut(&k) else { continue };
            c.on_timer(now_us);
            let pkt = c.poll_packet(now_us);
            let gone = c.removable(now_us);
            self.collect(&k);
            if gone { dead.push(k); }
            if let Some(pkt) = pkt { self.cursor = Some(k); result = Some((k.addr, pkt)); break; }
        }
        for k in dead { self.conns.remove(&k); }
        result
    }

    /// When `poll_transmit` next needs to run for timers.
    pub fn next_timeout(&self) -> Option<u64> {
        self.conns.values().filter_map(|c| c.next_deadline()).min()
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::Ip;
    use alloc::vec;

    const A: PeerAddr = PeerAddr { ip: Ip::V4([10, 0, 0, 1]), port: 1000 };
    const B: PeerAddr = PeerAddr { ip: Ip::V4([10, 0, 0, 2]), port: 2000 };
    const MS: u64 = 1000;

    #[test]
    fn header_golden_and_round_trip() {
        let h = Header { ty: PacketType::Syn, conn_id: 0x1234, timestamp_us: 0x0102_0304, timestamp_diff_us: 0, wnd_size: 0x0010_0000, seq_nr: 1, ack_nr: 0 };
        let b = encode(&h, None, &[]);
        assert_eq!(b, [0x41, 0, 0x12, 0x34, 1, 2, 3, 4, 0, 0, 0, 0, 0, 0x10, 0, 0, 0, 1, 0, 0]);
        let p = parse(&b).unwrap();
        assert_eq!(p.header, h);
        assert!(p.sack.is_none() && p.payload.is_empty());
        let h2 = Header { ty: PacketType::State, ..h };
        let b = encode(&h2, Some(&[0b101, 0, 0, 0x80]), b"xy");
        assert_eq!(b[1], 1);
        assert_eq!(&b[20..26], &[0, 4, 0b101, 0, 0, 0x80]);
        let p = parse(&b).unwrap();
        assert_eq!(p.sack, Some(&[0b101u8, 0, 0, 0x80][..]));
        assert_eq!(p.payload, b"xy");
        assert!(looks_like_utp(&b));
        assert!(!looks_like_utp(b"d1:ad2:id20:aaaaaaaaaaaaaaaaaaaae1:q4:ping1:t2:aa1:y1:qe"));
    }

    #[test]
    fn parse_hostile() {
        assert!(parse(&[]).is_none());
        assert!(parse(&[0x41; 19]).is_none());
        let mut b = encode(&Header { ty: PacketType::Data, conn_id: 1, timestamp_us: 0, timestamp_diff_us: 0, wnd_size: 0, seq_nr: 0, ack_nr: 0 }, None, &[]);
        b[0] = 0x42; // version 2
        assert!(parse(&b).is_none());
        b[0] = 0x51; // type 5
        assert!(parse(&b).is_none());
        b[0] = 0x01;
        b[1] = 1; // a SACK extension that is not there
        assert!(parse(&b).is_none());
        b.extend_from_slice(&[0, 3, 1, 2, 3]); // length not a multiple of 4
        assert!(parse(&b).is_none());
        b.truncate(20);
        b.extend_from_slice(&[7, 2, 0, 0]); // next ext 7 missing
        assert!(parse(&b).is_none());
        b.truncate(20);
        b[1] = 9; // unknown extension: skipped
        b.extend_from_slice(&[0, 2, 0xaa, 0xbb, b'z']);
        assert_eq!(parse(&b).unwrap().payload, b"z");
        // Fuzz: random bytes never panic, nor feeding them to a socket.
        let mut r = Rng(5);
        let mut s = UtpSocket::new(Config::default(), 1);
        for i in 0..20_000u64 {
            let n = (r.next() % 64) as usize;
            let mut v: Vec<u8> = (0..n).map(|_| r.next() as u8).collect();
            if i % 2 == 0 && !v.is_empty() { v[0] = (v[0] & 0x70) | 1; }
            let _ = parse(&v);
            s.on_datagram(A, &v, i);
            while s.poll_transmit(i).is_some() {}
        }
        assert!(s.len() <= Config::default().backlog);
    }

    /// An in-memory network: loss, delay, jitter (which reorders) and an optional
    /// bottleneck rate with a queue.
    struct Net {
        rng: Rng,
        loss_permille: u64,
        delay_us: u64,
        jitter_us: u64,
        rate_bytes_per_ms: Option<u64>,
        busy_until: [u64; 2],
        queue: Vec<(u64, u64, PeerAddr, PeerAddr, Vec<u8>)>,
        order: u64,
        drop_nth_data_from_a: Option<u64>,
        data_from_a: u64,
        max_queue_delay_us: u64,
    }

    impl Net {
        fn new(loss_permille: u64, delay_us: u64, jitter_us: u64) -> Net {
            Net { rng: Rng(99), loss_permille, delay_us, jitter_us, rate_bytes_per_ms: None, busy_until: [0; 2], queue: Vec::new(), order: 0, drop_nth_data_from_a: None, data_from_a: 0, max_queue_delay_us: 0 }
        }
        fn send(&mut self, now: u64, from: PeerAddr, to: PeerAddr, pkt: Vec<u8>) {
            if from == A && pkt[0] >> 4 == 0 {
                self.data_from_a += 1;
                if self.drop_nth_data_from_a == Some(self.data_from_a) { return; }
            }
            // SYNs are spared (connect timeouts have their own test).
            if pkt[0] >> 4 != 4 && self.rng.next() % 1000 < self.loss_permille { return; }
            let mut at = now;
            if let Some(rate) = self.rate_bytes_per_ms {
                let link = if from == A { 0 } else { 1 };
                let start = self.busy_until[link].max(now);
                self.busy_until[link] = start + pkt.len() as u64 * MS / rate;
                if from == A { self.max_queue_delay_us = self.max_queue_delay_us.max(start - now); }
                at = self.busy_until[link];
            }
            let jitter = if self.jitter_us > 0 { self.rng.next() % self.jitter_us } else { 0 };
            self.order += 1;
            self.queue.push((at + self.delay_us + jitter, self.order, from, to, pkt));
        }
        fn due(&mut self, now: u64) -> Vec<(PeerAddr, PeerAddr, Vec<u8>)> {
            self.queue.sort_by_key(|q| (q.0, q.1));
            let n = self.queue.iter().take_while(|q| q.0 <= now).count();
            self.queue.drain(..n).map(|q| (q.2, q.3, q.4)).collect()
        }
    }

    fn pump(net: &mut Net, a: &mut UtpSocket, b: &mut UtpSocket, now: u64) {
        for (from, to, pkt) in net.due(now) {
            if to == A { a.on_datagram(from, &pkt, now); } else { b.on_datagram(from, &pkt, now); }
        }
        while let Some((to, p)) = a.poll_transmit(now) { net.send(now, A, to, p); }
        while let Some((to, p)) = b.poll_transmit(now) { net.send(now, B, to, p); }
    }

    fn data(n: usize, seed: u64) -> Vec<u8> { let mut r = Rng(seed); (0..n).map(|_| r.next() as u8).collect() }

    /// Sends `ab` from A to B and `ba` from B to A, then both close. Returns the time used.
    fn transfer(net: &mut Net, cfg: Config, ab: &[u8], ba: &[u8], limit_us: u64, tweak: impl FnOnce(&mut UtpConn)) -> (u64, UtpSocket, UtpSocket) {
        let mut a = UtpSocket::new(cfg, 1);
        let mut b = UtpSocket::new(cfg, 2);
        let ka = a.connect(B, 0).unwrap();
        tweak(a.conn_mut(&ka).unwrap());
        let mut kb = None;
        let (mut sent_ab, mut sent_ba) = (0, 0);
        let (mut got_ab, mut got_ba) = (Vec::new(), Vec::new());
        let mut buf = vec![0u8; 4096];
        let mut closed = [false; 2];
        let mut closed_at = None;
        let mut now = 0;
        while now < limit_us {
            pump(net, &mut a, &mut b, now);
            if kb.is_none() { kb = b.accept(); }
            sent_ab += a.write(&ka, &ab[sent_ab..]);
            loop { let n = a.read(&ka, &mut buf); if n == 0 { break; } got_ba.extend_from_slice(&buf[..n]); }
            if let Some(kb) = kb {
                sent_ba += b.write(&kb, &ba[sent_ba..]);
                loop { let n = b.read(&kb, &mut buf); if n == 0 { break; } got_ab.extend_from_slice(&buf[..n]); }
                if sent_ba == ba.len() { if let Some(c) = b.conn_mut(&kb) { c.close(); } }
            }
            if sent_ab == ab.len() { if let Some(c) = a.conn_mut(&ka) { c.close(); } }
            while let Some(e) = a.poll_event() { if let Event::Closed(_, r) = e { assert_eq!(r, CloseReason::Graceful, "A at {} ms, got {}/{} sent {}/{}", now / MS, got_ba.len(), got_ab.len(), sent_ab, sent_ba); closed[0] = true; } }
            while let Some(e) = b.poll_event() { if let Event::Closed(_, r) = e { assert_eq!(r, CloseReason::Graceful, "B at {} ms, got {}/{} sent {}/{}", now / MS, got_ab.len(), got_ba.len(), sent_ab, sent_ba); closed[1] = true; } }
            if closed == [true, true] && closed_at.is_none() { closed_at = Some(now); }
            if closed == [true, true] && a.is_empty() && b.is_empty() { break; }
            now += MS;
        }
        assert_eq!(got_ab.len(), ab.len(), "A->B bytes at {} us", now);
        assert!(got_ab == ab, "A->B corrupted");
        assert!(got_ba == ba, "B->A corrupted");
        assert_eq!(closed, [true, true], "not closed by {} us", now);
        (closed_at.unwrap_or(now), a, b)
    }

    #[test]
    fn clean_transfer_both_ways_and_close() {
        let mut net = Net::new(0, 20 * MS, 0);
        let (t, a, b) = transfer(&mut net, Config::default(), &data(300_000, 1), &data(123_457, 2), 60_000 * MS, |_| {});
        assert!(t < 10_000 * MS, "took {} ms", t / MS);
        assert!(a.is_empty() && b.is_empty());
    }

    #[test]
    fn lossy_reordering_link() {
        for seed in 0..24 {
            let mut net = Net::new(100 + (seed % 12) * 10, 30 * MS, 40 * MS);
            net.rng = Rng(seed * 7 + 1);
            transfer(&mut net, Config::default(), &data(200_000, 3 + seed), &data(50_000, 4), 900_000 * MS, |_| {});
        }
    }

    #[test]
    fn sequence_numbers_wrap() {
        let mut net = Net::new(20, 10 * MS, 5 * MS);
        transfer(&mut net, Config::default(), &data(150_000, 5), &[], 300_000 * MS, |c| {
            // Start the stream just below the wrap point: ~110 packets cross it.
            c.inflight[0].seq = 0xffc0;
            c.seq_nr = 0xffc1;
            c.last_ack = 0xffbf;
        });
    }

    #[test]
    fn single_loss_recovers_fast() {
        let mut net = Net::new(0, 50 * MS, 0);
        net.drop_nth_data_from_a = Some(12);
        let (t, _, _) = transfer(&mut net, Config::default(), &data(100_000, 6), &[], 60_000 * MS, |_| {});
        // Recovery by duplicate / selective acks well before a 1 s timeout would; the
        // whole exchange (handshake, transfer, FINs) takes a handful of round trips.
        assert!(t < 1_500 * MS, "took {} ms", t / MS);
    }

    #[test]
    fn small_receive_window_and_slow_reader() {
        let cfg = Config { recv_buffer: 8 * 1024, ..Config::default() };
        let mut net = Net::new(0, 5 * MS, 0);
        let mut a = UtpSocket::new(cfg, 1);
        let mut b = UtpSocket::new(cfg, 2);
        let ka = a.connect(B, 0).unwrap();
        let payload = data(200_000, 7);
        let mut sent = 0;
        let mut got = Vec::new();
        let mut kb = None;
        let mut buf = [0u8; 512];
        let mut now = 0;
        while got.len() < payload.len() && now < 400_000 * MS {
            pump(&mut net, &mut a, &mut b, now);
            if kb.is_none() { kb = b.accept(); }
            sent += a.write(&ka, &payload[sent..]);
            // B reads only every 50 ms: the window closes and reopens repeatedly.
            if let Some(k) = kb { if now % (50 * MS) == 0 { let n = b.read(&k, &mut buf); got.extend_from_slice(&buf[..n]); } }
            now += MS;
            // Nothing B buffers exceeds its window.
            if let Some(k) = kb { assert!(b.conn(&k).unwrap().rx.len() <= 8 * 1024); }
        }
        assert!(got == payload);
        assert_eq!(a.conn(&ka).unwrap().state(), State::Connected);
    }

    #[test]
    fn connect_timeout() {
        let mut a = UtpSocket::new(Config::default(), 1);
        let k = a.connect(B, 0).unwrap();
        let mut syns = 0;
        let mut now = 0;
        let mut reason = None;
        while now < 30_000 * MS && reason.is_none() {
            while let Some((to, p)) = a.poll_transmit(now) { assert_eq!(to, B); assert_eq!(p[0], 0x41); syns += 1; }
            while let Some(e) = a.poll_event() { if let Event::Closed(kk, r) = e { assert_eq!(kk, k); reason = Some(r); } }
            if reason.is_none() { now = a.next_timeout().unwrap_or(now + MS).max(now + 1); }
        }
        assert_eq!(reason, Some(CloseReason::ConnectTimeout));
        assert_eq!(syns, MAX_SYN_ATTEMPTS);
        assert!(now < 10_000 * MS);
        assert!(a.poll_transmit(now).is_none());
        assert!(a.is_empty());
    }

    #[test]
    fn reset_for_unknown_and_reset_closes() {
        let mut b = UtpSocket::new(Config::default(), 2);
        let h = Header { ty: PacketType::Data, conn_id: 77, timestamp_us: 1, timestamp_diff_us: 0, wnd_size: 1000, seq_nr: 5, ack_nr: 0 };
        assert!(b.on_datagram(A, &encode(&h, None, b"hi"), 0));
        let (to, r) = b.poll_transmit(0).unwrap();
        assert_eq!(to, A);
        let p = parse(&r).unwrap();
        assert_eq!((p.header.ty, p.header.conn_id, p.header.ack_nr), (PacketType::Reset, 77, 5));
        // A RESET for nothing is not answered.
        assert!(b.on_datagram(A, &r, 0));
        assert!(b.poll_transmit(0).is_none());
        // A live connection is closed by a RESET with its send id.
        let mut net = Net::new(0, MS, 0);
        let mut a = UtpSocket::new(Config::default(), 1);
        let ka = a.connect(B, 0).unwrap();
        for t in 0..10 { pump(&mut net, &mut a, &mut b, t * MS); }
        let kb = b.accept().unwrap();
        assert_eq!(a.conn(&ka).unwrap().state(), State::Connected);
        b.conn_mut(&kb).unwrap().abort();
        for t in 10..20 { pump(&mut net, &mut a, &mut b, t * MS); }
        let mut evs = Vec::new();
        while let Some(e) = a.poll_event() { evs.push(e); }
        assert!(evs.contains(&Event::Closed(ka, CloseReason::Reset)), "{:?}", evs);
        assert!(a.is_empty() && b.is_empty());
    }

    #[test]
    fn backlog_is_bounded_and_duplicate_syn_reacked() {
        let cfg = Config { backlog: 2, ..Config::default() };
        let mut b = UtpSocket::new(cfg, 2);
        let syn = |id: u16| encode(&Header { ty: PacketType::Syn, conn_id: id, timestamp_us: 9, timestamp_diff_us: 0, wnd_size: 65536, seq_nr: 100, ack_nr: 0 }, None, &[]);
        b.on_datagram(A, &syn(10), 0);
        b.on_datagram(A, &syn(10), 0); // duplicate
        b.on_datagram(A, &syn(20), 0);
        b.on_datagram(A, &syn(30), 0); // over the backlog
        let mut out = Vec::new();
        while let Some((_, p)) = b.poll_transmit(0) { out.push(parse(&p).unwrap().header); }
        assert_eq!(b.len(), 2);
        assert!(out.iter().any(|h| h.ty == PacketType::Reset && h.conn_id == 30));
        let states: Vec<_> = out.iter().filter(|h| h.ty == PacketType::State).collect();
        assert_eq!(states.len(), 2);
        assert!(states.iter().all(|h| h.ack_nr == 100));
        assert!(states.iter().any(|h| h.conn_id == 10));
        b.on_datagram(A, &syn(10), 1);
        let (_, p) = b.poll_transmit(1).unwrap();
        assert_eq!(parse(&p).unwrap().header.ty, PacketType::State);
        assert_eq!(b.accept().map(|k| k.recv_id), Some(11));
        assert_eq!(b.accept().map(|k| k.recv_id), Some(21));
        assert_eq!(b.accept(), None);
    }

    #[test]
    fn hostile_acks_are_ignored() {
        let mut net = Net::new(0, MS, 0);
        let mut a = UtpSocket::new(Config::default(), 1);
        let mut b = UtpSocket::new(Config::default(), 2);
        let ka = a.connect(B, 0).unwrap();
        for t in 0..5 { pump(&mut net, &mut a, &mut b, t * MS); }
        a.write(&ka, &data(10_000, 1));
        let seq = a.conn(&ka).unwrap().seq_nr;
        // Forged acks for data never sent and a SACK pointing anywhere: nothing breaks.
        for ack in [0u16, 0x7fff, 0xffff, seq.wrapping_add(5)] {
            let h = Header { ty: PacketType::State, conn_id: ka.recv_id, timestamp_us: 0, timestamp_diff_us: u32::MAX, wnd_size: u32::MAX, seq_nr: 0, ack_nr: ack };
            a.on_datagram(B, &encode(&h, Some(&[0xff; 32]), &[]), 6 * MS);
        }
        for t in 6..3000 { pump(&mut net, &mut a, &mut b, t * MS); }
        let kb = b.accept().unwrap();
        let mut buf = vec![0u8; 20_000];
        assert_eq!(b.read(&kb, &mut buf), 10_000);
        assert_eq!(&buf[..10_000], &data(10_000, 1)[..]);
    }

    #[test]
    fn keepalive_and_idle_timeout() {
        let mut net = Net::new(0, MS, 0);
        let mut a = UtpSocket::new(Config::default(), 1);
        let mut b = UtpSocket::new(Config::default(), 2);
        let ka = a.connect(B, 0).unwrap();
        for t in 0..5 { pump(&mut net, &mut a, &mut b, t * MS); }
        // Idle for 3 minutes, stepping by timeouts: keepalives keep it open.
        let mut now = 5 * MS;
        while now < 180_000 * MS {
            pump(&mut net, &mut a, &mut b, now);
            now = [a.next_timeout(), b.next_timeout(), net.queue.iter().map(|q| q.0).min()].into_iter().flatten().min().unwrap_or(now + MS).max(now + 1);
        }
        assert_eq!(a.conn(&ka).unwrap().state(), State::Connected);
        // B vanishes: A gives up after the idle limit.
        let mut now2 = now;
        while now2 < now + 2 * IDLE_TIMEOUT_US && !a.is_empty() {
            while a.poll_transmit(now2).is_some() {}
            now2 = a.next_timeout().unwrap_or(now2 + MS).max(now2 + 1);
        }
        assert!(a.is_empty());
    }

    /// LEDBAT end to end: a 500 kB/s bottleneck with a deep buffer. uTP fills the link
    /// but keeps the queue it causes near the 100 ms target.
    #[test]
    fn does_not_swamp_a_bottleneck() {
        let mut net = Net::new(0, 10 * MS, 0);
        net.rate_bytes_per_ms = Some(500);
        let cfg = Config { send_buffer: 4 << 20, recv_buffer: 4 << 20, ..Config::default() };
        let mut a = UtpSocket::new(cfg, 1);
        let mut b = UtpSocket::new(cfg, 2);
        let ka = a.connect(B, 0).unwrap();
        let chunk = data(64 * 1024, 9);
        let mut got = 0usize;
        let mut buf = vec![0u8; 65536];
        let mut kb = None;
        let mut late_max = 0;
        for ms in 0..40_000u64 {
            let now = ms * MS;
            pump(&mut net, &mut a, &mut b, now);
            if kb.is_none() { kb = b.accept(); }
            if a.conn(&ka).unwrap().send_queue() < 1 << 20 { a.write(&ka, &chunk); }
            if let Some(k) = kb { loop { let n = b.read(&k, &mut buf); if n == 0 { break; } got += n; } }
            if ms == 15_000 { net.max_queue_delay_us = 0; }
            if ms > 15_000 { late_max = net.max_queue_delay_us; }
        }
        let rate = got as u64 / 40; // bytes per second over the run
        assert!(rate > 300_000, "underused the link: {} B/s", rate);
        assert!(late_max < 200 * MS, "queue delay reached {} ms", late_max / MS);
    }
}
