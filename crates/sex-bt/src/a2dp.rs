// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! A2DP media packets: SBC frames in RTP (A2DP 4.3.4, RFC 3550): a 12-byte RTP header
//! (version 2, payload type 96, sequence number, timestamp in samples, SSRC), one byte
//! saying how many SBC frames follow, then the frames. As many whole frames as the
//! transport channel's MTU takes, at most 15.

use crate::sbc::{self, Encoder};
use alloc::vec::Vec;

pub const RTP_HEADER: usize = 12;
pub const PAYLOAD_TYPE: u8 = 96;
pub const SSRC: u32 = 1;
/// The SBC media payload header holds a 4-bit frame count.
pub const MAX_FRAMES: usize = 15;

/// Encodes PCM into media packets for one stream.
pub struct Packetizer { enc: Encoder, seq: u16, timestamp: u32, per_packet: usize }

impl Packetizer {
    /// None when the coding is invalid or one frame does not fit `mtu`.
    pub fn new(config: sbc::Config, mtu: usize) -> Option<Packetizer> {
        let per_packet = (mtu.checked_sub(RTP_HEADER + 1)? / config.frame_bytes()).min(MAX_FRAMES);
        if per_packet == 0 { return None; }
        Some(Packetizer { enc: Encoder::new(config)?, seq: 0, timestamp: 0, per_packet })
    }
    pub fn config(&self) -> sbc::Config { self.enc.config }
    /// SBC frames per packet.
    pub fn frames_per_packet(&self) -> usize { self.per_packet }
    /// PCM frames (stereo pairs) one SBC frame takes.
    pub fn samples_per_frame(&self) -> usize { self.enc.config.samples() }

    /// One media packet from the front of `pcm`: as many whole SBC frames as it holds,
    /// up to a packet's worth. Returns (packet, pairs used); None when `pcm` is shorter
    /// than one frame.
    pub fn packet(&mut self, pcm: &[[i16; 2]]) -> Option<(Vec<u8>, usize)> {
        let n = self.samples_per_frame();
        let frames = (pcm.len() / n).min(self.per_packet);
        if frames == 0 { return None; }
        let mut p = Vec::with_capacity(RTP_HEADER + 1 + frames * self.enc.config.frame_bytes());
        p.extend_from_slice(&[0x80, PAYLOAD_TYPE]);
        p.extend_from_slice(&self.seq.to_be_bytes());
        p.extend_from_slice(&self.timestamp.to_be_bytes());
        p.extend_from_slice(&SSRC.to_be_bytes());
        p.push(frames as u8);
        for f in 0..frames { self.enc.encode(&pcm[f * n..(f + 1) * n], &mut p); }
        self.seq = self.seq.wrapping_add(1);
        self.timestamp = self.timestamp.wrapping_add((frames * n) as u32);
        Some((p, frames * n))
    }
}

/// A received media packet: (sequence, timestamp, frame count, SBC data), checked for a
/// version 2 header without padding or extension tricks we do not expect.
pub fn parse(p: &[u8]) -> Option<(u16, u32, u8, &[u8])> {
    if p.len() < RTP_HEADER + 1 || p[0] >> 6 != 2 { return None; }
    let csrc = (p[0] & 0x0F) as usize * 4;
    let ext = p[0] & 0x10 != 0;
    let mut at = RTP_HEADER + csrc;
    if ext {
        let len = u16::from_be_bytes([*p.get(at + 2)?, *p.get(at + 3)?]) as usize;
        at += 4 + 4 * len;
    }
    let mut end = p.len();
    if p[0] & 0x20 != 0 { end = end.checked_sub(*p.last()? as usize)?; }
    let head = *p.get(at)?;
    if at + 1 > end { return None; }
    Some((u16::from_be_bytes([p[2], p[3]]), u32::from_be_bytes([p[4], p[5], p[6], p[7]]), head & 0x0F, &p[at + 1..end]))
}
