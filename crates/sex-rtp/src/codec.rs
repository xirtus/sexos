// SPDX-License-Identifier: GPL-3.0-only
//! RTP payload formats: Opus (RFC 7587), VP8 (RFC 7741), VP9 (RFC 9628, non-flexible mode
//! only) and H.264 (RFC 6184 packetization-mode 1: single NAL, STAP-A, FU-A).
//!
//! A packetizer turns one encoded frame into payloads no larger than `mtu`; the last one
//! gets the RTP marker. A depacketizer takes payloads in sequence order (the jitter buffer
//! does the ordering) and hands back whole frames.
use alloc::vec::Vec;

pub trait Packetizer {
    fn packetize(&mut self, frame: &[u8], mtu: usize) -> Vec<Vec<u8>>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub data: Vec<u8>,
    pub keyframe: bool,
}

pub trait Depacketizer {
    /// One payload, in sequence order, with its RTP marker.
    fn push(&mut self, payload: &[u8], marker: bool) -> Option<Frame>;
}

/// Opus: one frame per packet, no payload header.
#[derive(Default)]
pub struct Opus;

impl Packetizer for Opus {
    fn packetize(&mut self, frame: &[u8], _mtu: usize) -> Vec<Vec<u8>> {
        alloc::vec![frame.into()]
    }
}
impl Depacketizer for Opus {
    fn push(&mut self, p: &[u8], _marker: bool) -> Option<Frame> {
        (!p.is_empty()).then(|| Frame { data: p.into(), keyframe: true })
    }
}

/// VP8 with a 15-bit PictureID in every packet, as browsers send it.
#[derive(Default)]
pub struct Vp8 {
    pub picture_id: u16,
    buf: Vec<u8>,
    started: bool,
    keyframe: bool,
}

impl Packetizer for Vp8 {
    fn packetize(&mut self, frame: &[u8], mtu: usize) -> Vec<Vec<u8>> {
        let room = mtu.saturating_sub(4).max(1);
        let pid = self.picture_id & 0x7fff;
        self.picture_id = self.picture_id.wrapping_add(1) & 0x7fff;
        frame
            .chunks(room)
            .enumerate()
            .map(|(i, c)| {
                let mut p = Vec::with_capacity(4 + c.len());
                p.push(0x80 | if i == 0 { 0x10 } else { 0 });
                p.push(0x80);
                p.push(0x80 | (pid >> 8) as u8);
                p.push(pid as u8);
                p.extend_from_slice(c);
                p
            })
            .collect()
    }
}

/// Bytes of the VP8 payload descriptor and its S bit / partition index.
pub fn vp8_descriptor(p: &[u8]) -> Option<(usize, bool, u8)> {
    let b0 = *p.first()?;
    let mut n = 1;
    if b0 & 0x80 != 0 {
        let x = *p.get(1)?;
        n = 2;
        if x & 0x80 != 0 {
            n += if *p.get(n)? & 0x80 != 0 { 2 } else { 1 };
        }
        if x & 0x40 != 0 {
            n += 1;
        }
        if x & 0x30 != 0 {
            n += 1;
        }
    }
    (p.len() >= n).then_some((n, b0 & 0x10 != 0, b0 & 0x07))
}

impl Depacketizer for Vp8 {
    fn push(&mut self, p: &[u8], marker: bool) -> Option<Frame> {
        let (n, s, pid) = vp8_descriptor(p)?;
        let data = &p[n..];
        if s && pid == 0 {
            self.buf.clear();
            self.started = true;
            // Frame tag bit 0 (P) clear means a key frame (RFC 6386 9.1).
            self.keyframe = data.first().map_or(false, |b| b & 1 == 0);
        }
        if !self.started {
            return None;
        }
        self.buf.extend_from_slice(data);
        if marker {
            self.started = false;
            return Some(Frame { data: core::mem::take(&mut self.buf), keyframe: self.keyframe });
        }
        None
    }
}

/// VP9 non-flexible mode: 15-bit PictureID, one spatial layer, scalability structure with
/// the resolution on key frames.
#[derive(Default)]
pub struct Vp9 {
    pub picture_id: u16,
    pub width: u16,
    pub height: u16,
    /// Set before `packetize` when the frame is a key frame.
    pub keyframe: bool,
    buf: Vec<u8>,
    started: bool,
    rx_key: bool,
}

impl Packetizer for Vp9 {
    fn packetize(&mut self, frame: &[u8], mtu: usize) -> Vec<Vec<u8>> {
        let pid = self.picture_id & 0x7fff;
        self.picture_id = self.picture_id.wrapping_add(1) & 0x7fff;
        let mut out = Vec::new();
        let mut off = 0;
        while off < frame.len() || out.is_empty() {
            let first = off == 0;
            let ss = first && self.keyframe;
            let hl = 3 + if ss { 1 + 4 } else { 0 };
            let n = (frame.len() - off).min(mtu.saturating_sub(hl).max(1));
            let last = off + n >= frame.len();
            let mut p = Vec::with_capacity(hl + n);
            // I P L F B E V Z
            p.push(0x80 | if self.keyframe { 0 } else { 0x40 } | if first { 0x08 } else { 0 } | if last { 0x04 } else { 0 } | if ss { 0x02 } else { 0 });
            p.push(0x80 | (pid >> 8) as u8);
            p.push(pid as u8);
            if ss {
                // N_S=0 (one layer), Y=1, G=0.
                p.push(0x10);
                p.extend_from_slice(&self.width.to_be_bytes());
                p.extend_from_slice(&self.height.to_be_bytes());
            }
            p.extend_from_slice(&frame[off..off + n]);
            out.push(p);
            off += n;
        }
        out
    }
}

/// VP9 payload descriptor: (header bytes, B, E, inter-predicted).
pub fn vp9_descriptor(p: &[u8]) -> Option<(usize, bool, bool, bool)> {
    let b0 = *p.first()?;
    let (i, pb, l, f, b, e, v) =
        (b0 & 0x80 != 0, b0 & 0x40 != 0, b0 & 0x20 != 0, b0 & 0x10 != 0, b0 & 0x08 != 0, b0 & 0x04 != 0, b0 & 0x02 != 0);
    let mut n = 1;
    if i {
        n += if *p.get(n)? & 0x80 != 0 { 2 } else { 1 };
    }
    if l {
        n += if f { 1 } else { 2 };
    }
    if f && pb {
        // Flexible mode reference indices.
        loop {
            let r = *p.get(n)?;
            n += 1;
            if r & 1 == 0 {
                break;
            }
        }
    }
    if v {
        let ss = *p.get(n)?;
        n += 1;
        let layers = (ss >> 5) as usize + 1;
        if ss & 0x10 != 0 {
            n += 4 * layers;
        }
        if ss & 0x08 != 0 {
            let groups = *p.get(n)? as usize;
            n += 1;
            for _ in 0..groups {
                let r = (*p.get(n)? >> 2) & 3;
                n += 1 + r as usize;
            }
        }
    }
    (p.len() >= n).then_some((n, b, e, pb))
}

impl Depacketizer for Vp9 {
    fn push(&mut self, p: &[u8], marker: bool) -> Option<Frame> {
        let (n, b, e, inter) = vp9_descriptor(p)?;
        if b {
            self.buf.clear();
            self.started = true;
            self.rx_key = !inter;
        }
        if !self.started {
            return None;
        }
        self.buf.extend_from_slice(&p[n..]);
        if e || marker {
            self.started = false;
            return Some(Frame { data: core::mem::take(&mut self.buf), keyframe: self.rx_key });
        }
        None
    }
}

/// H.264 packetization-mode 1. Frames are Annex B byte streams both ways.
#[derive(Default)]
pub struct H264 {
    buf: Vec<u8>,
    fu: Option<Vec<u8>>,
    keyframe: bool,
}

/// Split an Annex B stream into NAL units (start codes 00 00 01 or 00 00 00 01).
pub fn annexb_nals(d: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= d.len() {
        if d[i] == 0 && d[i + 1] == 0 && d[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut v = Vec::new();
    for (k, &s) in starts.iter().enumerate() {
        let mut e = starts.get(k + 1).map_or(d.len(), |&n| n - 3);
        while e > s && d[e - 1] == 0 && k + 1 < starts.len() {
            e -= 1;
        }
        if e > s {
            v.push(&d[s..e]);
        }
    }
    v
}

impl Packetizer for H264 {
    fn packetize(&mut self, frame: &[u8], mtu: usize) -> Vec<Vec<u8>> {
        let nals: Vec<&[u8]> = annexb_nals(frame).into_iter().filter(|n| !matches!(n[0] & 0x1f, 9)).collect();
        let mut out = Vec::new();
        let mut stap: Vec<&[u8]> = Vec::new();
        let flush = |stap: &mut Vec<&[u8]>, out: &mut Vec<Vec<u8>>| {
            match stap.len() {
                0 => {}
                1 => out.push(stap[0].into()),
                _ => {
                    let nri = stap.iter().map(|n| n[0] & 0x60).max().unwrap();
                    let mut p = Vec::from([nri | 24]);
                    for n in stap.iter() {
                        p.extend_from_slice(&(n.len() as u16).to_be_bytes());
                        p.extend_from_slice(n);
                    }
                    out.push(p);
                }
            }
            stap.clear();
        };
        for nal in nals {
            if nal.len() <= mtu {
                let size = 1 + stap.iter().map(|n| 2 + n.len()).sum::<usize>() + 2 + nal.len();
                if size > mtu {
                    flush(&mut stap, &mut out);
                }
                stap.push(nal);
                continue;
            }
            flush(&mut stap, &mut out);
            let (hdr, body) = (nal[0], &nal[1..]);
            let room = mtu.saturating_sub(2).max(1);
            let chunks: Vec<&[u8]> = body.chunks(room).collect();
            for (i, c) in chunks.iter().enumerate() {
                let mut p = Vec::with_capacity(2 + c.len());
                p.push(hdr & 0xe0 | 28);
                p.push(if i == 0 { 0x80 } else { 0 } | if i + 1 == chunks.len() { 0x40 } else { 0 } | hdr & 0x1f);
                p.extend_from_slice(c);
                out.push(p);
            }
        }
        flush(&mut stap, &mut out);
        out
    }
}

impl H264 {
    fn nal(&mut self, n: &[u8]) {
        if n.is_empty() {
            return;
        }
        if matches!(n[0] & 0x1f, 5 | 7) {
            self.keyframe = true;
        }
        self.buf.extend_from_slice(&[0, 0, 0, 1]);
        self.buf.extend_from_slice(n);
    }
}

impl Depacketizer for H264 {
    fn push(&mut self, p: &[u8], marker: bool) -> Option<Frame> {
        let t = p.first()? & 0x1f;
        match t {
            1..=23 => self.nal(p),
            24 => {
                let mut d = &p[1..];
                while d.len() >= 2 {
                    let n = u16::from_be_bytes([d[0], d[1]]) as usize;
                    if d.len() < 2 + n {
                        break;
                    }
                    let nal = d[2..2 + n].to_vec();
                    self.nal(&nal);
                    d = &d[2 + n..];
                }
            }
            28 if p.len() >= 2 => {
                let (s, e) = (p[1] & 0x80 != 0, p[1] & 0x40 != 0);
                if s {
                    let mut v = Vec::from([p[0] & 0xe0 | p[1] & 0x1f]);
                    v.extend_from_slice(&p[2..]);
                    self.fu = Some(v);
                } else if let Some(v) = self.fu.as_mut() {
                    v.extend_from_slice(&p[2..]);
                }
                if e {
                    if let Some(v) = self.fu.take() {
                        self.nal(&v);
                    }
                }
            }
            _ => {}
        }
        if marker && !self.buf.is_empty() {
            self.fu = None;
            let key = core::mem::take(&mut self.keyframe);
            return Some(Frame { data: core::mem::take(&mut self.buf), keyframe: key });
        }
        None
    }
}
