// SPDX-License-Identifier: GPL-3.0-only
//! Stage W4: finding networks. The firmware scans on its own (UMAC scan): the
//! driver sends one SCAN_REQ_UMAC listing the channels the NVM and regulatory
//! domain allow, the firmware hops through them and passes up every beacon and
//! probe response it hears as an RX MPDU, then reports the iteration and the
//! scan complete. Layouts follow OpenBSD iwx(4) (`iwx_umac_scan_v14`,
//! `iwx_fill_probe_req`, `iwx_rx_mpdu_mq`): request version 14, which also
//! serves the x17 firmware's version 15 (same layout).
use crate::bringup::Bringup;
use crate::cmdq::CommandQueue;
use crate::context::{Dma, DmaRead};
use crate::csr::{self, Registers};
use crate::devices::Family;
use crate::hcmd::{self as h, Packet};
use crate::ieee80211::{self as w, Bss};

pub const SCAN_REQ_UMAC: u8 = h::SCAN_REQ_UMAC;
pub const SCAN_COMPLETE_UMAC: u8 = 0x0f;
pub const SCAN_ITERATION_COMPLETE_UMAC: u8 = 0xb5;
pub const REPLY_RX_MPDU_CMD: u8 = 0xc1;
pub const REQUEST_V14_BYTES: usize = 1940;
/// Channels the firmware takes in one request (SCAN_CHANNEL_PARAMS v6).
pub const MAX_REQUEST_CHANNELS: usize = 67;
/// 2.4 and 5 GHz entries of the NVM channel table (6 GHz comes with W6).
pub const NVM_CHANNELS: [u8; 51] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14,
    36, 40, 44, 48, 52, 56, 60, 64, 68, 72, 76, 80, 84, 88, 92,
    96, 100, 104, 108, 112, 116, 120, 124, 128, 132, 136, 140, 144,
    149, 153, 157, 161, 165, 169, 173, 177, 181,
];
const TWO_GHZ: usize = 14;
pub const CHANNEL_VALID: u32 = 1 << 0;
pub const CHANNEL_ACTIVE: u32 = 1 << 3;
pub const MAX_NETWORKS: usize = 32;

const FLAGS_PASS_ALL: u16 = 1 << 1;
const FLAGS_NTFY_ITER_COMPLETE: u16 = 1 << 2;
const FLAGS_ADAPTIVE_DWELL: u16 = 1 << 7;
const FLAGS_FORCE_PASSIVE: u16 = 1 << 11;
const PRIORITY_EXT_6: u32 = 6;
const CHANNEL_FLAG_ENABLE_CHAN_ORDER: u8 = 1 << 5;
const PHY_BAND_5: u8 = 0;
const PHY_BAND_24: u8 = 1;
const MPDU_CRC_OK: u32 = 1 << 0;
const MPDU_OVERRUN_OK: u32 = 1 << 1;
const MPDU_PAD: u8 = 0x20;
const SCAN_US: u64 = 15_000_000;
const COMMAND_US: u64 = 1_000_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Channel { pub number: u8, pub five_ghz: bool, pub active: bool }

/// The channels to scan: valid entries of the NVM (or MCC) channel flags, 5 GHz
/// only when the SKU has the band (`iwx_init_channel_map`).
pub fn channels(flags: &[u32], has_5ghz: bool, out: &mut [Channel; NVM_CHANNELS.len()]) -> usize {
    let mut n = 0;
    for (i, (&f, &number)) in flags.iter().zip(NVM_CHANNELS.iter()).enumerate() {
        let five = i >= TWO_GHZ;
        if f & CHANNEL_VALID == 0 || (five && !has_5ghz) { continue; }
        out[n] = Channel { number, five_ghz: five, active: f & CHANNEL_ACTIVE != 0 };
        n += 1;
    }
    n
}

/// What the request depends on besides the channels.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub mac: [u8; 6],
    /// Firmware API 58: channel entries carry a band byte.
    pub ext_channel: bool,
    /// Capability 9: the firmware fills in a DS parameter element.
    pub ds_param: bool,
    pub has_5ghz: bool,
    /// The firmware's channel limit (TLV N_SCAN_CHANNELS).
    pub max_channels: usize,
}

/// SCAN_REQ_UMAC v14/v15: a one-shot passive scan of `channels` for every network.
pub fn request_v14(out: &mut [u8; REQUEST_V14_BYTES], channels: &[Channel], o: &Options) -> usize {
    out.fill(0);
    let put16 = |b: &mut [u8], at: usize, v: u16| b[at..at + 2].copy_from_slice(&v.to_le_bytes());
    let put32 = |b: &mut [u8], at: usize, v: u32| b[at..at + 4].copy_from_slice(&v.to_le_bytes());
    put32(out, 4, PRIORITY_EXT_6);
    // General parameters (v10) at 8.
    let g = 8;
    put16(out, g, FLAGS_FORCE_PASSIVE | FLAGS_PASS_ALL | FLAGS_NTFY_ITER_COMPLETE | FLAGS_ADAPTIVE_DWELL);
    out[g + 4] = 10; out[g + 5] = 10; // active dwell, both LMACs (TU)
    out[g + 6] = 2; out[g + 7] = 8; out[g + 8] = 10; // adaptive dwell: APs per 2.4 GHz, 5 GHz, social channel
    put16(out, g + 10, 300); // adaptive dwell budget for a full scan (TU)
    put32(out, g + 28, PRIORITY_EXT_6);
    out[g + 32] = 110; out[g + 33] = 110; // passive dwell (TU)
    // Channel parameters (v6) at 44.
    let c = 44;
    out[c] = CHANNEL_FLAG_ENABLE_CHAN_ORDER;
    out[c + 2] = 10; out[c + 3] = 2; // adaptive dwell overrides: GO-friendly, social
    let limit = MAX_REQUEST_CHANNELS.min(o.max_channels);
    let mut count = 0;
    for ch in channels.iter().take(limit) {
        let at = c + 4 + count * 8;
        // flags (directed-SSID bitmap) stay 0: no SSID is probed for.
        if o.ext_channel {
            out[at + 4] = ch.number;
            out[at + 5] = if ch.five_ghz { PHY_BAND_5 } else { PHY_BAND_24 };
            out[at + 6] = 1;
        } else {
            out[at + 4] = ch.number;
            out[at + 5] = 1;
        }
        count += 1;
    }
    out[c + 1] = count as u8;
    // Periodic parameters at 584: one iteration, now.
    out[584 + 2] = 1;
    // Probe parameters at 596: the probe request template (unused while passive).
    let p = 596;
    let buf = p + 20;
    let frame = &mut out[buf..buf + 512];
    frame[0] = 0x40; // management, probe request
    frame[4..10].fill(0xff);
    frame[10..16].copy_from_slice(&o.mac);
    frame[16..22].fill(0xff);
    // SSID element (empty; the firmware inserts one when probing directed).
    let mut at = 26;
    put16(out, p + 2, at as u16); // mac_header: offset 0, length 26
    let band24 = at;
    let add = |out: &mut [u8], at: &mut usize, bytes: &[u8]| { out[buf + *at..buf + *at + bytes.len()].copy_from_slice(bytes); *at += bytes.len(); };
    add(out, &mut at, &[1, 8, 2, 4, 11, 22, 12, 18, 24, 36]);
    add(out, &mut at, &[50, 4, 48, 72, 96, 108]);
    if o.ds_param { add(out, &mut at, &[3, 1, 0]); }
    put16(out, p + 4, band24 as u16);
    put16(out, p + 6, (at - band24) as u16);
    if o.has_5ghz {
        let band5 = at;
        add(out, &mut at, &[1, 8, 12, 18, 24, 36, 48, 72, 96, 108]);
        put16(out, p + 8, band5 as u16);
        put16(out, p + 10, (at - band5) as u16);
    }
    put16(out, p + 16, at as u16); // common data: empty
    REQUEST_V14_BYTES
}

/// A frame the firmware passed up, with what the receiver measured.
pub struct Received<'a> { pub frame: &'a [u8], pub rssi: i16, pub channel: u8, pub status: u32 }

/// Reads an RX MPDU notification (`iwx_rx_mpdu_mq`): the descriptor, then the
/// 802.11 frame. A padded header is joined to its body in `scratch`.
pub fn parse_mpdu<'a>(p: &'a Packet<'_>, family: Family, scratch: &'a mut [u8; 4096]) -> Option<Received<'a>> {
    if !p.is(h::LEGACY_GROUP, REPLY_RX_MPDU_CMD) { return None; }
    let d = p.data;
    let desc = if family == Family::Ax210 { 64 } else { 48 };
    if d.len() < desc { return None; }
    let len = u16::from_le_bytes([d[0], d[1]]) as usize;
    let status = u32::from_le_bytes(d[12..16].try_into().unwrap());
    if status & MPDU_CRC_OK == 0 || status & MPDU_OVERRUN_OK == 0 || len < 24 || len > d.len() - desc { return None; }
    let (ea, eb, channel) = if family == Family::Ax210 { (d[40], d[41], d[42]) } else { (d[32], d[33], d[34]) };
    let energy = |e: u8| if e == 0 { -256 } else { -(e as i16) };
    let rssi = energy(ea).max(energy(eb));
    let body = &d[desc..desc + len];
    // With the pad flag, two bytes sit between the header and the body
    // (`iwx_rx_mpdu_mq` removes them for any frame type).
    let header = match body[0] & 0x0c { 0 => Some(24), 8 => crate::ieee80211::data_header_len(body), _ => None };
    let frame: &[u8] = match header {
        Some(h) if d[3] & MPDU_PAD != 0 => {
            if len < h + 2 { return None; }
            scratch[..h].copy_from_slice(&body[..h]);
            scratch[h..len - 2].copy_from_slice(&body[h + 2..]);
            &scratch[..len - 2]
        }
        _ => body,
    };
    Some(Received { frame, rssi, channel, status })
}

/// One network heard, keeping the strongest signal.
#[derive(Clone, Copy, Debug)]
pub struct Network { pub bss: Bss, pub rssi: i16, pub heard_on: u8, pub frames: u16 }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step { Start, Requested, Scanning, Done, Failed(Failure) }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure { Refused, NoAnswer, Timeout, Crashed { cause: u32 }, BadRxBuffer(u16), Status(u8), Busy }

pub struct Scan {
    pub step: Step,
    pub family: Family,
    pub networks: [Option<Network>; MAX_NETWORKS],
    pub count: usize,
    /// Networks not kept because the list was full.
    pub dropped: u32,
    pub frames: u32,
    pub channels: u8,
    pub status: Option<u8>,
    since: u64,
    request: [u8; REQUEST_V14_BYTES],
    len: usize,
}

impl Scan {
    pub fn new(family: Family, channels: &[Channel], options: &Options, now: u64) -> Self {
        let mut request = [0u8; REQUEST_V14_BYTES];
        let len = request_v14(&mut request, channels, options);
        Self { step: Step::Start, family, networks: [None; MAX_NETWORKS], count: 0, dropped: 0, frames: 0,
            channels: request[45], status: None, since: now, request, len }
    }

    pub fn done(&self) -> bool { matches!(self.step, Step::Done | Step::Failed(_)) }
    pub fn networks(&self) -> impl Iterator<Item = &Network> { self.networks.iter().flatten() }

    /// Adds or refreshes a network from a beacon or probe response.
    pub fn heard(&mut self, frame: &[u8], rssi: i16, channel: u8) {
        let Some(bss) = w::parse_bss(frame) else { return };
        self.frames = self.frames.saturating_add(1);
        if let Some(n) = self.networks.iter_mut().flatten().find(|n| n.bss.bssid == bss.bssid) {
            n.frames = n.frames.saturating_add(1);
            if rssi > n.rssi { n.rssi = rssi; n.bss = bss; n.heard_on = channel; }
            return;
        }
        match self.networks.iter_mut().find(|n| n.is_none()) {
            Some(slot) => { *slot = Some(Network { bss, rssi, heard_on: channel, frames: 1 }); self.count += 1; }
            None => {
                // Full: replace the weakest if this one is stronger.
                let weakest = self.networks.iter_mut().flatten().min_by_key(|n| n.rssi).unwrap();
                if weakest.rssi < rssi { *weakest = Network { bss, rssi, heard_on: channel, frames: 1 }; }
                self.dropped = self.dropped.saturating_add(1);
            }
        }
    }

    pub fn on_packet(&mut self, p: &Packet<'_>, cmdq: &mut CommandQueue) {
        if cmdq.on_packet(p) { return; }
        let mut scratch = [0u8; 4096];
        if let Some(r) = parse_mpdu(p, self.family, &mut scratch) {
            self.heard(r.frame, r.rssi, r.channel);
        } else if p.is(h::LONG_GROUP, SCAN_ITERATION_COMPLETE_UMAC) && p.data.len() >= 8 {
            self.channels = p.data[4];
        } else if p.is(h::LONG_GROUP, SCAN_COMPLETE_UMAC) && p.data.len() >= 8 {
            self.status = Some(p.data[6]);
        }
    }

    /// One step. Returns true while the scan runs.
    pub fn advance<D: Dma + DmaRead>(&mut self, b: &mut Bringup, cmdq: &mut CommandQueue, r: &mut impl Registers, dma: &mut D, now: u64) -> bool {
        if self.done() { return false; }
        let cause = r.read(csr::INT);
        if cause != u32::MAX && cause & (csr::INT_SW_ERR | csr::INT_HW_ERR) != 0 { self.step = Step::Failed(Failure::Crashed { cause }); return false; }
        if cause != 0 && cause != u32::MAX { r.write(csr::INT, cause); }
        if let Err(id) = b.receive(r, dma, |p| self.on_packet(p, cmdq)) { self.step = Step::Failed(Failure::BadRxBuffer(id)); return false; }
        let waited = now.saturating_sub(self.since);
        match self.step {
            Step::Start => match cmdq.send(dma, r, &b.layout, b.base, h::LONG_GROUP, SCAN_REQ_UMAC, 0, &self.request[..self.len]) {
                Ok(_) => { self.step = Step::Requested; self.since = now; }
                Err(_) => self.step = Step::Failed(Failure::Busy),
            },
            Step::Requested => {
                if let Some(a) = cmdq.answer.take() {
                    self.step = if a.failed { Step::Failed(Failure::Refused) } else { Step::Scanning };
                    self.since = now;
                } else if waited >= COMMAND_US { cmdq.abandon(); self.step = Step::Failed(Failure::NoAnswer); }
            }
            Step::Scanning => {
                // Status 1 is "completed" (SCAN_COMP_STATUS_SUCCESS); others are aborts.
                match self.status {
                    Some(1) => self.step = Step::Done,
                    Some(s) => self.step = Step::Failed(Failure::Status(s)),
                    None if waited >= SCAN_US => self.step = Step::Failed(Failure::Timeout),
                    None => {}
                }
            }
            _ => {}
        }
        !self.done()
    }
}
