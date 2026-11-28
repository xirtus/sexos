// SPDX-License-Identifier: GPL-3.0-only
//! Receive side: a jitter buffer that orders packets, hands back whole frames, asks for
//! missing packets with NACK, and gives up on a frame after `max_delay_ms`; and the send
//! side's history so NACKed packets can be sent again.
use crate::packet::{Packet, SeqUnwrapper};
use alloc::collections::{BTreeMap, VecDeque};
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug)]
pub struct JitterConfig {
    /// How long a gap may hold up later frames.
    pub max_delay_ms: u64,
    /// Audio: every packet is a frame. Video: a frame ends at the marker.
    pub frame_per_packet: bool,
    /// NACK attempts per missing packet; 0 turns NACK off.
    pub max_nacks: u32,
    /// Packets kept at most.
    pub capacity: usize,
}

impl JitterConfig {
    pub fn video() -> JitterConfig {
        JitterConfig { max_delay_ms: 500, frame_per_packet: false, max_nacks: 10, capacity: 2048 }
    }
    pub fn audio() -> JitterConfig {
        JitterConfig { max_delay_ms: 120, frame_per_packet: true, max_nacks: 0, capacity: 256 }
    }
}

struct Missing {
    last_nack: Option<u64>,
    tries: u32,
}

pub struct JitterBuffer {
    cfg: JitterConfig,
    unwrap: SeqUnwrapper,
    packets: BTreeMap<u64, (Packet, u64)>,
    next: Option<u64>,
    delivered: bool,
    missing: BTreeMap<u64, Missing>,
    /// Round-trip time for NACK spacing.
    pub rtt_ms: u64,
    /// Frames given up on; the receiver should send a PLI when this grows.
    pub lost_frames: u64,
    pub late_packets: u64,
}

impl JitterBuffer {
    pub fn new(cfg: JitterConfig) -> JitterBuffer {
        JitterBuffer {
            cfg,
            unwrap: SeqUnwrapper::default(),
            packets: BTreeMap::new(),
            next: None,
            delivered: false,
            missing: BTreeMap::new(),
            rtt_ms: 100,
            lost_frames: 0,
            late_packets: 0,
        }
    }

    pub fn push(&mut self, p: Packet, now_ms: u64) {
        let s = self.unwrap.unwrap(p.seq);
        let mut next = *self.next.get_or_insert(s);
        if s < next && !self.delivered && next - s < 512 {
            // Before the first frame goes out, the stream starts at the lowest seen.
            if self.cfg.max_nacks > 0 {
                for m in s + 1..next {
                    if !self.packets.contains_key(&m) {
                        self.missing.insert(m, Missing { last_nack: None, tries: 0 });
                    }
                }
            }
            self.next = Some(s);
            next = s;
        }
        if s < next || self.packets.contains_key(&s) {
            self.late_packets += 1;
            return;
        }
        let top = self.packets.keys().next_back().copied().map_or(next, |t| t + 1).max(next);
        if self.cfg.max_nacks > 0 && s > top {
            for m in top..s.min(top + 512) {
                self.missing.insert(m, Missing { last_nack: None, tries: 0 });
            }
        }
        self.missing.remove(&s);
        self.packets.insert(s, (p, now_ms));
        while self.packets.len() > self.cfg.capacity {
            let (k, _) = self.packets.pop_first().unwrap();
            self.next = Some(k + 1);
        }
    }

    /// The next complete frame's packets, in order.
    pub fn pop(&mut self, now_ms: u64) -> Option<Vec<Packet>> {
        loop {
            let n = self.next?;
            if self.cfg.frame_per_packet {
                if let Some((p, _)) = self.packets.remove(&n) {
                    self.next = Some(n + 1);
                    self.delivered = true;
                    return Some(alloc::vec![p]);
                }
            } else {
                let mut end = None;
                let mut k = n;
                while let Some((p, _)) = self.packets.get(&k) {
                    if p.marker {
                        end = Some(k);
                        break;
                    }
                    k += 1;
                }
                if let Some(e) = end {
                    let v = (n..=e).map(|k| self.packets.remove(&k).unwrap().0).collect();
                    self.next = Some(e + 1);
                    self.delivered = true;
                    return Some(v);
                }
            }
            // Head incomplete: wait, unless the oldest packet has waited too long.
            let oldest = self.packets.values().map(|v| v.1).min()?;
            if now_ms.saturating_sub(oldest) < self.cfg.max_delay_ms {
                return None;
            }
            self.skip_frame(n);
        }
    }

    /// Drop the broken frame at the head: everything up to its marker.
    fn skip_frame(&mut self, n: u64) {
        let first = *self.packets.keys().next().unwrap();
        let e = if self.cfg.frame_per_packet {
            first - 1
        } else {
            // Through the broken frame's marker; without one, all we have of it.
            self.packets.iter().find(|(_, v)| v.0.marker).map(|(k, _)| *k).unwrap_or(*self.packets.keys().next_back().unwrap())
        };
        let keep = self.packets.split_off(&(e + 1));
        self.packets = keep;
        self.missing = self.missing.split_off(&(e + 1));
        self.next = Some((e + 1).max(n));
        self.delivered = true;
        self.lost_frames += 1;
    }

    /// Sequence numbers to NACK now.
    pub fn nacks(&mut self, now_ms: u64) -> Vec<u16> {
        let next = self.next.unwrap_or(0);
        self.missing = self.missing.split_off(&next);
        let (rtt, max) = (self.rtt_ms.max(5), self.cfg.max_nacks);
        let mut v = Vec::new();
        self.missing.retain(|&s, m| {
            if m.last_nack.map_or(true, |t| now_ms >= t + rtt) {
                if m.tries >= max {
                    return false;
                }
                m.tries += 1;
                m.last_nack = Some(now_ms);
                v.push(s as u16);
            }
            true
        });
        v
    }
}

/// One outgoing RTP stream: numbering, counters for sender reports, and a history for
/// retransmission on NACK.
pub struct RtpStream {
    pub ssrc: u32,
    pub payload_type: u8,
    pub seq: u16,
    pub packets_sent: u32,
    pub octets_sent: u32,
    history: VecDeque<Packet>,
    history_len: usize,
}

impl RtpStream {
    pub fn new(ssrc: u32, payload_type: u8, first_seq: u16) -> RtpStream {
        RtpStream { ssrc, payload_type, seq: first_seq, packets_sent: 0, octets_sent: 0, history: VecDeque::new(), history_len: 512 }
    }

    /// Wrap a frame's payloads in RTP packets; the last gets the marker.
    pub fn packets(&mut self, payloads: Vec<Vec<u8>>, timestamp: u32) -> Vec<Packet> {
        let n = payloads.len();
        let v: Vec<Packet> = payloads
            .into_iter()
            .enumerate()
            .map(|(i, payload)| {
                let p = Packet {
                    marker: i + 1 == n,
                    payload_type: self.payload_type,
                    seq: self.seq,
                    timestamp,
                    ssrc: self.ssrc,
                    payload,
                    ..Packet::default()
                };
                self.seq = self.seq.wrapping_add(1);
                self.packets_sent = self.packets_sent.wrapping_add(1);
                self.octets_sent = self.octets_sent.wrapping_add(p.payload.len() as u32);
                p
            })
            .collect();
        for p in &v {
            if self.history.len() == self.history_len {
                self.history.pop_front();
            }
            self.history.push_back(p.clone());
        }
        v
    }

    /// A sent packet again, for a NACK.
    pub fn resend(&self, seq: u16) -> Option<&Packet> {
        self.history.iter().find(|p| p.seq == seq)
    }

    pub fn sender_report(&self, ntp: u64, rtp_ts: u32) -> crate::rtcp::Rtcp {
        crate::rtcp::Rtcp::SenderReport {
            ssrc: self.ssrc,
            ntp,
            rtp_ts,
            packets: self.packets_sent,
            octets: self.octets_sent,
            reports: Vec::new(),
        }
    }
}
