// SPDX-License-Identifier: GPL-3.0-only
//! RTCP (RFC 3550 6) and the feedback WebRTC uses: generic NACK (RFC 4585), PLI, FIR
//! (RFC 5104), REMB (draft-alvestrand-rmcat-remb) and transport-wide congestion control
//! feedback (draft-holmer-rmcat-transport-wide-cc-extensions-01).
use crate::packet::Error;
use alloc::string::String;
use alloc::vec::Vec;

pub const PT_SR: u8 = 200;
pub const PT_RR: u8 = 201;
pub const PT_SDES: u8 = 202;
pub const PT_BYE: u8 = 203;
pub const PT_RTPFB: u8 = 205;
pub const PT_PSFB: u8 = 206;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReportBlock {
    pub ssrc: u32,
    pub fraction_lost: u8,
    /// 24-bit signed.
    pub cumulative_lost: i32,
    pub highest_seq: u32,
    pub jitter: u32,
    pub last_sr: u32,
    pub delay_since_last_sr: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransportCc {
    pub sender_ssrc: u32,
    pub media_ssrc: u32,
    pub base_seq: u16,
    /// Multiples of 64 ms, 24-bit signed.
    pub reference_time: i32,
    pub fb_count: u8,
    /// Per packet from base_seq: arrival delta from the previous received packet (or the
    /// reference time) in 250 us units, None when not received.
    pub deltas: Vec<Option<i32>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rtcp {
    SenderReport { ssrc: u32, ntp: u64, rtp_ts: u32, packets: u32, octets: u32, reports: Vec<ReportBlock> },
    ReceiverReport { ssrc: u32, reports: Vec<ReportBlock> },
    /// (ssrc, CNAME) chunks; other items are skipped.
    Sdes(Vec<(u32, String)>),
    Bye { ssrcs: Vec<u32>, reason: Option<String> },
    /// (pid, blp) pairs.
    Nack { sender_ssrc: u32, media_ssrc: u32, pairs: Vec<(u16, u16)> },
    Pli { sender_ssrc: u32, media_ssrc: u32 },
    /// (ssrc, command sequence number).
    Fir { sender_ssrc: u32, entries: Vec<(u32, u8)> },
    Remb { sender_ssrc: u32, bitrate: u64, ssrcs: Vec<u32> },
    TransportCc(TransportCc),
    Other { pt: u8, fmt: u8, body: Vec<u8> },
}

fn be16(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}
fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn header(v: &mut Vec<u8>, count: u8, pt: u8) -> usize {
    let at = v.len();
    v.extend_from_slice(&[0x80 | count, pt, 0, 0]);
    at
}
fn finish(v: &mut Vec<u8>, at: usize) {
    while (v.len() - at) % 4 != 0 {
        v.push(0);
    }
    let words = ((v.len() - at) / 4 - 1) as u16;
    v[at + 2..at + 4].copy_from_slice(&words.to_be_bytes());
}
fn put32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_be_bytes());
}

fn write_block(v: &mut Vec<u8>, r: &ReportBlock) {
    put32(v, r.ssrc);
    put32(v, (r.fraction_lost as u32) << 24 | (r.cumulative_lost as u32 & 0xff_ffff));
    put32(v, r.highest_seq);
    put32(v, r.jitter);
    put32(v, r.last_sr);
    put32(v, r.delay_since_last_sr);
}

fn read_block(b: &[u8]) -> ReportBlock {
    let w = be32(&b[4..]);
    ReportBlock {
        ssrc: be32(b),
        fraction_lost: (w >> 24) as u8,
        cumulative_lost: ((w << 8) as i32) >> 8,
        highest_seq: be32(&b[8..]),
        jitter: be32(&b[12..]),
        last_sr: be32(&b[16..]),
        delay_since_last_sr: be32(&b[20..]),
    }
}

impl Rtcp {
    /// Parse a compound packet.
    pub fn parse_compound(mut b: &[u8]) -> Result<Vec<Rtcp>, Error> {
        let mut out = Vec::new();
        while !b.is_empty() {
            if b.len() < 4 || b[0] >> 6 != 2 {
                return Err(Error::Malformed);
            }
            let len = (be16(&b[2..]) as usize + 1) * 4;
            if b.len() < len {
                return Err(Error::Malformed);
            }
            let mut end = len;
            if b[0] & 0x20 != 0 {
                let p = b[len - 1] as usize;
                if p == 0 || p > len - 4 {
                    return Err(Error::Malformed);
                }
                end -= p;
            }
            out.push(Self::parse_one(b[0] & 31, b[1], &b[4..end])?);
            b = &b[len..];
        }
        Ok(out)
    }

    fn parse_one(count: u8, pt: u8, d: &[u8]) -> Result<Rtcp, Error> {
        let need = |n: usize| if d.len() < n { Err(Error::Malformed) } else { Ok(()) };
        let blocks = |at: usize| -> Result<Vec<ReportBlock>, Error> {
            if d.len() < at + 24 * count as usize {
                return Err(Error::Malformed);
            }
            Ok((0..count as usize).map(|i| read_block(&d[at + 24 * i..])).collect())
        };
        Ok(match (pt, count) {
            (PT_SR, _) => {
                need(24)?;
                Rtcp::SenderReport {
                    ssrc: be32(d),
                    ntp: (be32(&d[4..]) as u64) << 32 | be32(&d[8..]) as u64,
                    rtp_ts: be32(&d[12..]),
                    packets: be32(&d[16..]),
                    octets: be32(&d[20..]),
                    reports: blocks(24)?,
                }
            }
            (PT_RR, _) => {
                need(4)?;
                Rtcp::ReceiverReport { ssrc: be32(d), reports: blocks(4)? }
            }
            (PT_SDES, _) => {
                let mut v = Vec::new();
                let mut i = 0;
                for _ in 0..count {
                    if d.len() < i + 4 {
                        return Err(Error::Malformed);
                    }
                    let ssrc = be32(&d[i..]);
                    i += 4;
                    let mut cname = String::new();
                    while i < d.len() && d[i] != 0 {
                        if i + 2 > d.len() || i + 2 + d[i + 1] as usize > d.len() {
                            return Err(Error::Malformed);
                        }
                        let (t, l) = (d[i], d[i + 1] as usize);
                        if t == 1 {
                            cname = String::from_utf8_lossy(&d[i + 2..i + 2 + l]).into();
                        }
                        i += 2 + l;
                    }
                    i = (i + 4) & !3;
                    v.push((ssrc, cname));
                }
                Rtcp::Sdes(v)
            }
            (PT_BYE, _) => {
                need(4 * count as usize)?;
                let ssrcs = (0..count as usize).map(|i| be32(&d[4 * i..])).collect();
                let at = 4 * count as usize;
                let reason = (d.len() > at && d.len() >= at + 1 + d[at] as usize)
                    .then(|| String::from_utf8_lossy(&d[at + 1..at + 1 + d[at] as usize]).into());
                Rtcp::Bye { ssrcs, reason }
            }
            (PT_RTPFB, 1) => {
                need(8)?;
                let pairs = d[8..].chunks_exact(4).map(|c| (be16(c), be16(&c[2..]))).collect();
                Rtcp::Nack { sender_ssrc: be32(d), media_ssrc: be32(&d[4..]), pairs }
            }
            (PT_RTPFB, 15) => Rtcp::TransportCc(parse_twcc(d)?),
            (PT_PSFB, 1) => {
                need(8)?;
                Rtcp::Pli { sender_ssrc: be32(d), media_ssrc: be32(&d[4..]) }
            }
            (PT_PSFB, 4) => {
                need(8)?;
                let entries = d[8..].chunks_exact(8).map(|c| (be32(c), c[4])).collect();
                Rtcp::Fir { sender_ssrc: be32(d), entries }
            }
            (PT_PSFB, 15) if d.len() >= 16 && &d[8..12] == b"REMB" => {
                let n = d[12] as usize;
                let exp = d[13] >> 2;
                let mant = ((d[13] & 3) as u64) << 16 | (d[14] as u64) << 8 | d[15] as u64;
                need(16 + 4 * n)?;
                let ssrcs = (0..n).map(|i| be32(&d[16 + 4 * i..])).collect();
                Rtcp::Remb { sender_ssrc: be32(d), bitrate: mant << exp, ssrcs }
            }
            _ => Rtcp::Other { pt, fmt: count, body: d.into() },
        })
    }

    /// Append this packet to a compound packet.
    pub fn write(&self, v: &mut Vec<u8>) {
        match self {
            Rtcp::SenderReport { ssrc, ntp, rtp_ts, packets, octets, reports } => {
                let at = header(v, reports.len() as u8, PT_SR);
                put32(v, *ssrc);
                put32(v, (ntp >> 32) as u32);
                put32(v, *ntp as u32);
                put32(v, *rtp_ts);
                put32(v, *packets);
                put32(v, *octets);
                reports.iter().for_each(|r| write_block(v, r));
                finish(v, at);
            }
            Rtcp::ReceiverReport { ssrc, reports } => {
                let at = header(v, reports.len() as u8, PT_RR);
                put32(v, *ssrc);
                reports.iter().for_each(|r| write_block(v, r));
                finish(v, at);
            }
            Rtcp::Sdes(chunks) => {
                let at = header(v, chunks.len() as u8, PT_SDES);
                for (ssrc, cname) in chunks {
                    let c0 = v.len();
                    put32(v, *ssrc);
                    v.push(1);
                    v.push(cname.len().min(255) as u8);
                    v.extend_from_slice(&cname.as_bytes()[..cname.len().min(255)]);
                    v.push(0);
                    while (v.len() - c0) % 4 != 0 {
                        v.push(0);
                    }
                }
                finish(v, at);
            }
            Rtcp::Bye { ssrcs, reason } => {
                let at = header(v, ssrcs.len() as u8, PT_BYE);
                ssrcs.iter().for_each(|s| put32(v, *s));
                if let Some(r) = reason {
                    v.push(r.len().min(255) as u8);
                    v.extend_from_slice(&r.as_bytes()[..r.len().min(255)]);
                }
                finish(v, at);
            }
            Rtcp::Nack { sender_ssrc, media_ssrc, pairs } => {
                let at = header(v, 1, PT_RTPFB);
                put32(v, *sender_ssrc);
                put32(v, *media_ssrc);
                for (p, b) in pairs {
                    v.extend_from_slice(&p.to_be_bytes());
                    v.extend_from_slice(&b.to_be_bytes());
                }
                finish(v, at);
            }
            Rtcp::Pli { sender_ssrc, media_ssrc } => {
                let at = header(v, 1, PT_PSFB);
                put32(v, *sender_ssrc);
                put32(v, *media_ssrc);
                finish(v, at);
            }
            Rtcp::Fir { sender_ssrc, entries } => {
                let at = header(v, 4, PT_PSFB);
                put32(v, *sender_ssrc);
                put32(v, 0);
                for (s, n) in entries {
                    put32(v, *s);
                    v.extend_from_slice(&[*n, 0, 0, 0]);
                }
                finish(v, at);
            }
            Rtcp::Remb { sender_ssrc, bitrate, ssrcs } => {
                let at = header(v, 15, PT_PSFB);
                put32(v, *sender_ssrc);
                put32(v, 0);
                v.extend_from_slice(b"REMB");
                let mut exp = 0u8;
                while bitrate >> exp > 0x3ffff {
                    exp += 1;
                }
                let m = (bitrate >> exp) as u32;
                v.extend_from_slice(&[ssrcs.len() as u8, exp << 2 | (m >> 16) as u8, (m >> 8) as u8, m as u8]);
                ssrcs.iter().for_each(|s| put32(v, *s));
                finish(v, at);
            }
            Rtcp::TransportCc(t) => {
                let at = header(v, 15, PT_RTPFB);
                write_twcc(v, t);
                // Padding is signalled with P and a count byte.
                let pad = (4 - (v.len() - at) % 4) % 4;
                if pad > 0 {
                    v.extend(core::iter::repeat(0).take(pad - 1));
                    v.push(pad as u8);
                    v[at] |= 0x20;
                }
                finish(v, at);
            }
            Rtcp::Other { pt, fmt, body } => {
                let at = header(v, *fmt, *pt);
                v.extend_from_slice(body);
                finish(v, at);
            }
        }
    }

    pub fn compound(packets: &[Rtcp]) -> Vec<u8> {
        let mut v = Vec::new();
        packets.iter().for_each(|p| p.write(&mut v));
        v
    }
}

/// Compress a list of loss bits into NACK (pid, blp) pairs.
pub fn nack_pairs(seqs: &[u16]) -> Vec<(u16, u16)> {
    let mut out: Vec<(u16, u16)> = Vec::new();
    for &s in seqs {
        if let Some((pid, blp)) = out.last_mut() {
            let d = s.wrapping_sub(*pid);
            if (1..=16).contains(&d) {
                *blp |= 1 << (d - 1);
                continue;
            }
        }
        out.push((s, 0));
    }
    out
}

/// Expand NACK pairs to sequence numbers.
pub fn nack_seqs(pairs: &[(u16, u16)]) -> Vec<u16> {
    let mut v = Vec::new();
    for &(pid, blp) in pairs {
        v.push(pid);
        for i in 0..16 {
            if blp & (1 << i) != 0 {
                v.push(pid.wrapping_add(i + 1));
            }
        }
    }
    v
}

fn parse_twcc(d: &[u8]) -> Result<TransportCc, Error> {
    if d.len() < 16 {
        return Err(Error::Malformed);
    }
    let base_seq = be16(&d[8..]);
    let count = be16(&d[10..]) as usize;
    let rt = (d[12] as u32) << 16 | (d[13] as u32) << 8 | d[14] as u32;
    let reference_time = ((rt << 8) as i32) >> 8;
    let fb_count = d[15];
    let mut i = 16;
    let mut sym: Vec<u8> = Vec::with_capacity(count);
    while sym.len() < count {
        if i + 2 > d.len() {
            return Err(Error::Malformed);
        }
        let c = be16(&d[i..]);
        i += 2;
        if c & 0x8000 == 0 {
            let s = ((c >> 13) & 3) as u8;
            let n = (c & 0x1fff) as usize;
            sym.extend(core::iter::repeat(s).take(n.min(count - sym.len())));
        } else if c & 0x4000 == 0 {
            for k in (0..14).rev() {
                if sym.len() < count {
                    sym.push(((c >> k) & 1) as u8);
                }
            }
        } else {
            for k in (0..7).rev() {
                if sym.len() < count {
                    sym.push(((c >> (2 * k)) & 3) as u8);
                }
            }
        }
    }
    let mut deltas = Vec::with_capacity(count);
    for s in sym {
        deltas.push(match s {
            0 => None,
            1 => {
                let v = *d.get(i).ok_or(Error::Malformed)? as i32;
                i += 1;
                Some(v)
            }
            2 => {
                if i + 2 > d.len() {
                    return Err(Error::Malformed);
                }
                let v = be16(&d[i..]) as i16 as i32;
                i += 2;
                Some(v)
            }
            _ => return Err(Error::Malformed),
        });
    }
    Ok(TransportCc { sender_ssrc: be32(d), media_ssrc: be32(&d[4..]), base_seq, reference_time, fb_count, deltas })
}

fn symbol(d: &Option<i32>) -> u8 {
    match d {
        None => 0,
        Some(v) if (0..=255).contains(v) => 1,
        Some(_) => 2,
    }
}

fn write_twcc(v: &mut Vec<u8>, t: &TransportCc) {
    put32(v, t.sender_ssrc);
    put32(v, t.media_ssrc);
    v.extend_from_slice(&t.base_seq.to_be_bytes());
    v.extend_from_slice(&(t.deltas.len() as u16).to_be_bytes());
    let rt = t.reference_time as u32 & 0xff_ffff;
    v.extend_from_slice(&[(rt >> 16) as u8, (rt >> 8) as u8, rt as u8, t.fb_count]);
    let syms: Vec<u8> = t.deltas.iter().map(symbol).collect();
    let mut i = 0;
    while i < syms.len() {
        // A run when at least 7 symbols repeat or the rest is one symbol, else a two-bit
        // status vector of 7.
        let run = syms[i..].iter().take_while(|&&s| s == syms[i]).count().min(0x1fff);
        if run >= 7 || i + run == syms.len() {
            v.extend_from_slice(&(((syms[i] as u16) << 13) | run as u16).to_be_bytes());
            i += run;
        } else {
            let mut c = 0xc000u16;
            for k in 0..7 {
                let s = *syms.get(i + k).unwrap_or(&0) as u16;
                c |= s << (2 * (6 - k));
            }
            v.extend_from_slice(&c.to_be_bytes());
            i += 7;
        }
    }
    for d in &t.deltas {
        match symbol(d) {
            1 => v.push(d.unwrap() as u8),
            2 => v.extend_from_slice(&(d.unwrap() as i16).to_be_bytes()),
            _ => {}
        }
    }
}

/// Builds transport-cc feedback from arrival times on the receiving side.
#[derive(Default)]
pub struct TwccRecorder {
    unwrap: crate::packet::SeqUnwrapper,
    /// (extended transport seq, arrival us)
    arrivals: alloc::collections::BTreeMap<u64, u64>,
    next_base: Option<u64>,
    fb_count: u8,
}

impl TwccRecorder {
    pub fn record(&mut self, seq: u16, arrival_us: u64) {
        let s = self.unwrap.unwrap(seq);
        if self.next_base.map_or(true, |b| s >= b) {
            self.arrivals.entry(s).or_insert(arrival_us);
        }
    }

    /// Feedback for everything recorded since the last call.
    pub fn feedback(&mut self, sender_ssrc: u32, media_ssrc: u32) -> Option<TransportCc> {
        let (&first, &first_t) = self.arrivals.iter().next()?;
        let base = self.next_base.map_or(first, |b| b.min(first));
        let last = *self.arrivals.keys().next_back().unwrap();
        let reference_time = (first_t / 64_000) as i32;
        let mut prev = reference_time as i64 * 64_000;
        let mut deltas = Vec::new();
        for s in base..=last {
            match self.arrivals.get(&s) {
                Some(&t) => {
                    let mut q = (t as i64 - prev) / 250;
                    q = q.clamp(i16::MIN as i64, i16::MAX as i64);
                    prev += q * 250;
                    deltas.push(Some(q as i32));
                }
                None => deltas.push(None),
            }
        }
        self.arrivals.clear();
        self.next_base = Some(last + 1);
        let fb = TransportCc {
            sender_ssrc,
            media_ssrc,
            base_seq: base as u16,
            reference_time,
            fb_count: self.fb_count,
            deltas,
        };
        self.fb_count = self.fb_count.wrapping_add(1);
        Some(fb)
    }
}

impl TransportCc {
    /// (transport seq, arrival in us relative to the reference clock or None).
    pub fn arrivals(&self) -> Vec<(u16, Option<i64>)> {
        let mut t = self.reference_time as i64 * 64_000;
        self.deltas
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let s = self.base_seq.wrapping_add(i as u16);
                match d {
                    Some(d) => {
                        t += *d as i64 * 250;
                        (s, Some(t))
                    }
                    None => (s, None),
                }
            })
            .collect()
    }
}

/// Receive statistics for one source (RFC 3550 A.3 and A.8): loss and interarrival jitter
/// for receiver reports.
#[derive(Clone, Debug, Default)]
pub struct ReceiveStats {
    unwrap: crate::packet::SeqUnwrapper,
    base: Option<u64>,
    max: u64,
    received: u64,
    expected_prior: u64,
    received_prior: u64,
    transit: Option<i64>,
    /// Interarrival jitter times 16 (RFC 3550 A.8).
    jitter16: u64,
    last_sr: u32,
    last_sr_at_ms: Option<u64>,
}

impl ReceiveStats {
    /// One packet: its sequence number, RTP timestamp and arrival in clock-rate units.
    pub fn packet(&mut self, seq: u16, rtp_ts: u32, arrival_ts: u32) {
        let s = self.unwrap.unwrap(seq);
        if self.base.is_none() {
            self.base = Some(s);
            self.max = s;
        }
        self.max = self.max.max(s);
        self.received += 1;
        let transit = arrival_ts.wrapping_sub(rtp_ts) as i32 as i64;
        if let Some(t) = self.transit {
            let d = (transit - t).unsigned_abs();
            self.jitter16 = self.jitter16 + d - ((self.jitter16 + 8) >> 4);
        }
        self.transit = Some(transit);
    }

    /// A sender report arrived: remember its middle NTP bits for LSR/DLSR.
    pub fn sender_report(&mut self, ntp: u64, now_ms: u64) {
        self.last_sr = (ntp >> 16) as u32;
        self.last_sr_at_ms = Some(now_ms);
    }

    pub fn report(&mut self, ssrc: u32, now_ms: u64) -> ReportBlock {
        let base = self.base.unwrap_or(0);
        let expected = if self.base.is_some() { self.max - base + 1 } else { 0 };
        let lost = expected as i64 - self.received as i64;
        let exp_int = expected - self.expected_prior;
        let rec_int = self.received - self.received_prior;
        self.expected_prior = expected;
        self.received_prior = self.received;
        let lost_int = exp_int as i64 - rec_int as i64;
        let fraction = if exp_int == 0 || lost_int <= 0 { 0 } else { ((lost_int << 8) / exp_int as i64) as u8 };
        ReportBlock {
            ssrc,
            fraction_lost: fraction,
            cumulative_lost: lost.clamp(-0x80_0000, 0x7f_ffff) as i32,
            highest_seq: (self.max.saturating_sub(0x1_0000)) as u32,
            jitter: (self.jitter16 >> 4) as u32,
            last_sr: self.last_sr,
            delay_since_last_sr: self.last_sr_at_ms.map_or(0, |t| ((now_ms - t) * 65536 / 1000) as u32),
        }
    }
}
