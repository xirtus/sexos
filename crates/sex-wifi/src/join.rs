// SPDX-License-Identifier: GPL-3.0-only
//! Stage W5a: getting onto a network's channel, as iwx(4) does in `iwx_auth`
//! before the first authentication frame: a PHY context on the AP's channel, the
//! receive-chain setting (RLC), a MAC context for our station, the binding of
//! the two, a station entry for the AP, the TX queue for management frames, and
//! a "session protection" time event so the firmware stays on the channel.
//!
//! Every command is built here byte for byte (the layouts are iwx's
//! `iwx_phy_context_cmd_uhb`, `iwx_rlc_config_cmd`, `iwx_mac_ctx_cmd`,
//! `iwx_binding_cmd`, `iwx_add_sta_cmd`, `iwx_scd_queue_cfg_cmd`,
//! `iwx_session_prot_cmd`) and checked in host tests. Chips that only speak the
//! newer "MLD" API (BZ family, MA MAC) are not handled; the x17's TY chip is not one.
use crate::bringup::Bringup;
use crate::cmdq::{self, CommandQueue};
use crate::context::{Dma, DmaRead, TFD_CB_SIZE};
use crate::csr::{self, Registers};
use crate::hcmd::{self as h, Packet};
use crate::ieee80211::Bss;
use crate::init::{Failure, Features};
use crate::ucode::{self as u, Firmware};

pub const PHY_ID: u32 = 0;
pub const MAC_ID: u32 = 0;
pub const STATION_ID: u8 = 0;
/// Non-QoS frames use this TID and the first queue after the command queue.
pub const MGMT_TID: u8 = 15;
pub const MGMT_QUEUE: u16 = 1;

pub const ACTION_ADD: u32 = 1;
pub const ACTION_MODIFY: u32 = 2;
pub const ACTION_REMOVE: u32 = 3;

const MAC_TYPE_BSS_STA: u32 = 5;
const MAC_TYPE_MA: u32 = 0x44;
const TSF_ID_A: u32 = 0;
const MAC_FLG_SHORT_SLOT: u32 = 1 << 4;
const MAC_FLG_SHORT_PREAMBLE: u32 = 1 << 5;
const MAC_FILTER_ACCEPT_GRP: u32 = 1 << 2;
const MAC_FILTER_IN_BEACON: u32 = 1 << 6;
const PHY_BAND_5: u8 = 0;
const PHY_BAND_24: u8 = 1;
const PHY_RX_CHAIN_VALID_POS: u32 = 1;
const PHY_RX_CHAIN_CNT_POS: u32 = 10;
const PHY_RX_CHAIN_MIMO_CNT_POS: u32 = 12;
const LMAC_24G_INDEX: u32 = 0;
const LMAC_5G_INDEX: u32 = 1;
const FW_CTXT_INVALID: u32 = 0xffff_ffff;
const STA_FLG_FAT_EN_MSK: u32 = 3 << 26;
const STA_FLG_MIMO_EN_MSK: u32 = 3 << 28;
const STA_LINK: u8 = 0;
const ADD_STA_SUCCESS: u32 = 1;
const SCD_QUEUE_ADD: u32 = 0;
const SESSION_PROTECT_CONF_ASSOC: u32 = 0;
const TX_QUEUE_CFG_ENABLE_QUEUE: u16 = 1 << 0;

pub const PHY_CONTEXT_BYTES: usize = 32; // with the 6 GHz-sized channel info
pub const PHY_CONTEXT_V1_BYTES: usize = 28;
pub const RLC_BYTES: usize = 32;
pub const MAC_CONTEXT_BYTES: usize = 148;
pub const BINDING_BYTES: usize = 28;
pub const ADD_STA_BYTES: usize = 48;
pub const SCD_QUEUE_BYTES: usize = 36;
pub const TX_QUEUE_V0_BYTES: usize = 24;
pub const SESSION_PROTECTION_BYTES: usize = 24;

fn put32(p: &mut [u8], at: usize, v: u32) { p[at..at + 4].copy_from_slice(&v.to_le_bytes()); }
fn put16(p: &mut [u8], at: usize, v: u16) { p[at..at + 2].copy_from_slice(&v.to_le_bytes()); }
fn put64(p: &mut [u8], at: usize, v: u64) { p[at..at + 8].copy_from_slice(&v.to_le_bytes()); }

/// The AP to join and what the commands need to know about it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    pub bssid: [u8; 6],
    pub channel: u8,
    pub beacon_interval: u16,
    pub dtim_period: u8,
    pub basic_cck: u8,
    pub basic_ofdm: u8,
    pub short_slot: bool,
    pub short_preamble: bool,
}
impl Target {
    pub fn of(bss: &Bss, heard_on: u8) -> Self {
        Self {
            bssid: bss.bssid, channel: if bss.channel != 0 { bss.channel } else { heard_on },
            beacon_interval: bss.beacon_interval, dtim_period: bss.dtim_period,
            basic_cck: bss.basic_cck, basic_ofdm: bss.basic_ofdm,
            short_slot: bss.capability & 0x0400 != 0, short_preamble: bss.capability & 0x0020 != 0,
        }
    }
    pub fn two_ghz(&self) -> bool { self.channel <= 14 }
}

/// The rates control frames may answer at (`iwx_ack_rates`): the AP's basic rates
/// plus every mandatory rate below the lowest of them. Returns (CCK, OFDM) bits.
pub fn ack_rates(t: &Target) -> (u32, u32) {
    let lowest = |bits: u8| if bits == 0 { None } else { Some(bits.trailing_zeros()) };
    let mut cck = if t.two_ghz() { t.basic_cck as u32 & 0xf } else { 0 };
    let mut ofdm = t.basic_ofdm as u32;
    // OFDM: 24 Mb/s is index 4 of OFDM, 12 Mb/s index 2, 6 Mb/s index 0.
    if let Some(l) = lowest(ofdm as u8) {
        if 4 < l { ofdm |= 1 << 4; }
        if 2 < l { ofdm |= 1 << 2; }
    }
    ofdm |= 1;
    // CCK: 11 Mb/s is index 3, 5.5 index 2, 2 index 1, 1 index 0.
    if let Some(l) = lowest(cck as u8) {
        if 3 < l { cck |= 1 << 3; }
        if 2 < l { cck |= 1 << 2; }
        if 1 < l { cck |= 1 << 1; }
    }
    cck |= 1;
    (cck, ofdm)
}

/// The receive-chain word (`iwx_fw_valid_rx_ant`, one idle and one active chain).
pub fn rx_chain_info(rx_antennas: u8) -> u32 {
    (rx_antennas as u32) << PHY_RX_CHAIN_VALID_POS | 1 << PHY_RX_CHAIN_CNT_POS | 1 << PHY_RX_CHAIN_MIMO_CNT_POS
}

/// The LMAC a context runs on: the 5 GHz one only on dual-band (CDB) firmware
/// for a 5 GHz channel, as iwx's PHY and binding commands choose it.
pub fn lmac(channel: u8, cdb: bool) -> u32 {
    if channel <= 14 || !cdb { LMAC_24G_INDEX } else { LMAC_5G_INDEX }
}

/// PHY_CONTEXT_CMD v3/v4 for a 20 MHz channel. `uhb`: the firmware has the 6 GHz
/// channel numbering (capa 48), which widens the channel field to 32 bits.
/// `rx_chain`: only when RLC_CONFIG_CMD is not version 2 (it then carries it).
pub fn phy_context(out: &mut [u8; PHY_CONTEXT_BYTES], action: u32, channel: u8, uhb: bool, cdb: bool, rx_chain: Option<u32>) -> usize {
    out.fill(0);
    put32(out, 0, PHY_ID);
    put32(out, 4, action);
    let band = if channel <= 14 { PHY_BAND_24 } else { PHY_BAND_5 };
    // Width 20 MHz (0) and control position "1 below" (0) stay zero.
    let after = if uhb {
        put32(out, 8, channel as u32);
        out[12] = band;
        16
    } else {
        out[8] = band;
        out[9] = channel;
        12
    };
    put32(out, after, lmac(channel, cdb));
    if let Some(info) = rx_chain { put32(out, after + 4, info); }
    if uhb { PHY_CONTEXT_BYTES } else { PHY_CONTEXT_V1_BYTES }
}

/// RLC_CONFIG_CMD v2 (`iwx_phy_send_rlc`).
pub fn rlc(out: &mut [u8; RLC_BYTES], rx_chain: u32) {
    out.fill(0);
    put32(out, 0, PHY_ID);
    put32(out, 4, rx_chain);
}

/// EDCA parameters per access category in iwx's order (BE, BK, VI, VO): ECWmin,
/// ECWmax, AIFSN, TXOP limit in 32 µs units (802.11's defaults for an OFDM BSS,
/// as net80211 sets them before the AP's own arrive).
const EDCA: [(u8, u8, u8, u16); 4] = [(4, 10, 3, 0), (4, 10, 7, 0), (3, 4, 2, 94), (2, 3, 2, 47)];
/// TX FIFO per access category (`iwx_ac_to_tx_fifo`).
const AC_FIFO: [usize; 4] = [2, 1, 3, 4];

/// MAC_CONTEXT_CMD for our station (`iwx_mac_ctxt_cmd`). `aid` is Some after
/// association; until then beacons are let through.
pub fn mac_context(out: &mut [u8; MAC_CONTEXT_BYTES], action: u32, me: &[u8; 6], t: &Target, aid: Option<u16>) {
    out.fill(0);
    put32(out, 0, MAC_ID);
    put32(out, 4, action);
    if action == ACTION_REMOVE { return; }
    put32(out, 8, MAC_TYPE_BSS_STA);
    put32(out, 12, TSF_ID_A);
    out[16..22].copy_from_slice(me);
    out[24..30].copy_from_slice(&t.bssid);
    let (cck, ofdm) = ack_rates(t);
    put32(out, 32, cck);
    put32(out, 36, ofdm);
    // protection_flags at 40 stay 0 (no HT yet; W6).
    put32(out, 44, if t.short_preamble { MAC_FLG_SHORT_PREAMBLE } else { 0 });
    put32(out, 48, if t.short_slot { MAC_FLG_SHORT_SLOT } else { 0 });
    let assoc = aid.filter(|&a| a != 0 && t.dtim_period != 0);
    put32(out, 52, MAC_FILTER_ACCEPT_GRP | if assoc.is_none() { MAC_FILTER_IN_BEACON } else { 0 });
    // qos_flags at 56 stay 0 (QoS is negotiated in W6), then ac[5] from 60.
    for (ac, &(ecw_min, ecw_max, aifsn, txop)) in EDCA.iter().enumerate() {
        let fifo = AC_FIFO[ac];
        let at = 60 + fifo * 8;
        put16(out, at, (1u16 << ecw_min) - 1);
        put16(out, at + 2, (1u16 << ecw_max) - 1);
        out[at + 4] = aifsn;
        out[at + 5] = 1 << fifo;
        put16(out, at + 6, txop * 32);
    }
    // Station data from 100 (`iwx_mac_ctxt_cmd_fill_sta`); DTIM timing is the
    // firmware's to learn from beacons, as is_assoc stays 0 until then.
    let sta = 100;
    put32(out, sta, aid.is_some() as u32);
    put32(out, sta + 16, t.beacon_interval as u32);
    put32(out, sta + 24, t.beacon_interval as u32 * t.dtim_period.max(1) as u32);
    put32(out, sta + 32, 10); // listen interval
    put32(out, sta + 36, aid.map_or(0, |a| (a & 0x3fff) as u32));
}

/// BINDING_CONTEXT_CMD v2: our MAC context on PHY context 0, on the same LMAC.
pub fn binding(out: &mut [u8; BINDING_BYTES], action: u32, channel: u8, cdb: bool) {
    out.fill(0);
    put32(out, 0, PHY_ID);
    put32(out, 4, action);
    put32(out, 8, MAC_ID);
    put32(out, 12, FW_CTXT_INVALID);
    put32(out, 16, FW_CTXT_INVALID);
    put32(out, 20, PHY_ID);
    put32(out, 24, lmac(channel, cdb));
}

/// ADD_STA for the AP (`iwx_add_sta_cmd`, no HT yet). `update` modifies the
/// existing entry (after association) instead of adding it.
pub fn add_station(out: &mut [u8; ADD_STA_BYTES], bssid: &[u8; 6], update: bool) {
    out.fill(0);
    out[0] = update as u8;
    put32(out, 4, MAC_ID);
    if !update { out[8..14].copy_from_slice(bssid); }
    out[16] = STATION_ID;
    put32(out, 24, STA_FLG_FAT_EN_MSK | STA_FLG_MIMO_EN_MSK);
    out[35] = STA_LINK;
}

/// SCD_QUEUE_CONFIG_CMD v3 (operation ADD) for the management queue.
pub fn queue_add(out: &mut [u8; SCD_QUEUE_BYTES], ring: u64, byte_counts: u64) {
    out.fill(0);
    put32(out, 0, SCD_QUEUE_ADD);
    put32(out, 4, 1 << STATION_ID);
    out[8] = MGMT_TID;
    put32(out, 16, TFD_CB_SIZE);
    put64(out, 20, byte_counts);
    put64(out, 28, ring);
}

/// The older TX_QUEUE_CFG (`iwx_tx_queue_cfg_cmd`, LONG_GROUP 0xd8) for firmware
/// without SCD_QUEUE_CONFIG v3.
pub fn queue_add_v0(out: &mut [u8; TX_QUEUE_V0_BYTES], ring: u64, byte_counts: u64) {
    out.fill(0);
    out[0] = STATION_ID;
    out[1] = MGMT_TID;
    put16(out, 2, TX_QUEUE_CFG_ENABLE_QUEUE);
    put32(out, 4, TFD_CB_SIZE);
    put64(out, 8, byte_counts);
    put64(out, 16, ring);
}
pub const TX_QUEUE_CFG_V0: u8 = 0x1d;

/// SESSION_PROTECTION_CMD: stay on the channel for `duration_tu` while associating.
pub fn session_protection(out: &mut [u8; SESSION_PROTECTION_BYTES], action: u32, duration_tu: u32) {
    out.fill(0);
    put32(out, 0, MAC_ID);
    put32(out, 4, action);
    put32(out, 8, SESSION_PROTECT_CONF_ASSOC);
    put32(out, 12, duration_tu);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step { Phy, Rlc, Mac, Binding, Station, Queue, Protect, Joined, Failed(Failure) }

/// Firmware facts for joining.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Versions { pub phy: u8, pub rlc: u8, pub scd_queue: u8, pub uhb: bool, pub cdb: bool, pub mld: bool }
impl Versions {
    pub fn of(fw: &Firmware<'_>, features: &Features, hw_rev: u32) -> Self {
        let v = |g, c| fw.command_version(g, c).map_or(0, |v| v.version);
        // iwx uses the MLD API only on MA-type MACs (0x44, AX211 in some laptops)
        // and the BZ family; the x17's TY chip (0x42) keeps the older commands.
        let ma = (hw_rev >> 4) & 0xfff == MAC_TYPE_MA;
        Self {
            phy: v(h::LONG_GROUP, h::PHY_CONTEXT_CMD), rlc: v(h::DATA_PATH_GROUP, h::RLC_CONFIG_CMD),
            scd_queue: v(h::DATA_PATH_GROUP, h::SCD_QUEUE_CONFIG_CMD),
            uhb: features.capa(u::CAPA_ULTRA_HB_CHANNELS),
            cdb: features.capa(u::CAPA_CDB_SUPPORT),
            mld: features.capa(u::CAPA_MLD_API_SUPPORT) && ma,
        }
    }
}

const COMMAND_US: u64 = 1_000_000;

pub struct Join {
    pub step: Step,
    pub target: Target,
    pub versions: Versions,
    me: [u8; 6],
    rx_antennas: u8,
    /// Status word of the last BINDING / ADD_STA answer, kept for the log.
    pub status: u32,
    since: u64,
    sent: bool,
}

impl Join {
    pub fn new(target: Target, versions: Versions, me: [u8; 6], rx_antennas: u8, now: u64) -> Self {
        let step = if versions.mld { Step::Failed(Failure::Unsupported("mld_api")) }
            else if versions.phy != 3 && versions.phy != 4 { Step::Failed(Failure::Unsupported("phy_context_version")) }
            else { Step::Phy };
        Self { step, target, versions, me, rx_antennas, status: 0, since: now, sent: false }
    }
    pub fn done(&self) -> bool { matches!(self.step, Step::Joined | Step::Failed(_)) }
    fn go(&mut self, step: Step, now: u64) { self.step = step; self.since = now; self.sent = false; }
    fn next(&self, step: Step) -> Step {
        match step {
            Step::Phy if self.versions.rlc == 2 => Step::Rlc,
            Step::Phy | Step::Rlc => Step::Mac,
            Step::Mac => Step::Binding,
            Step::Binding => Step::Station,
            Step::Station => Step::Queue,
            Step::Queue => Step::Protect,
            Step::Protect | Step::Joined => Step::Joined,
            Step::Failed(f) => Step::Failed(f),
        }
    }

    /// The command for a step: (group, opcode, length).
    fn command(&self, step: Step, b: &Bringup, p: &mut [u8; MAC_CONTEXT_BYTES]) -> (u8, u8, usize) {
        let rx = rx_chain_info(self.rx_antennas);
        match step {
            Step::Phy => {
                let mut c = [0u8; PHY_CONTEXT_BYTES];
                let n = phy_context(&mut c, ACTION_ADD, self.target.channel, self.versions.uhb, self.versions.cdb, (self.versions.rlc != 2).then_some(rx));
                p[..n].copy_from_slice(&c[..n]);
                (h::LONG_GROUP, h::PHY_CONTEXT_CMD, n)
            }
            Step::Rlc => { let mut c = [0u8; RLC_BYTES]; rlc(&mut c, rx); p[..RLC_BYTES].copy_from_slice(&c); (h::DATA_PATH_GROUP, h::RLC_CONFIG_CMD, RLC_BYTES) }
            Step::Mac => { mac_context(p, ACTION_ADD, &self.me, &self.target, None); (h::LONG_GROUP, h::MAC_CONTEXT_CMD, MAC_CONTEXT_BYTES) }
            Step::Binding => { let mut c = [0u8; BINDING_BYTES]; binding(&mut c, ACTION_ADD, self.target.channel, self.versions.cdb); p[..BINDING_BYTES].copy_from_slice(&c); (h::LONG_GROUP, h::BINDING_CONTEXT_CMD, BINDING_BYTES) }
            Step::Station => { let mut c = [0u8; ADD_STA_BYTES]; add_station(&mut c, &self.target.bssid, false); p[..ADD_STA_BYTES].copy_from_slice(&c); (h::LONG_GROUP, h::ADD_STA, ADD_STA_BYTES) }
            Step::Queue => {
                let (ring, counts) = (b.base + b.layout.tx_ring.offset, b.base + b.layout.tx_byte_counts.offset);
                if self.versions.scd_queue == 3 {
                    let mut c = [0u8; SCD_QUEUE_BYTES]; queue_add(&mut c, ring, counts); p[..SCD_QUEUE_BYTES].copy_from_slice(&c);
                    (h::DATA_PATH_GROUP, h::SCD_QUEUE_CONFIG_CMD, SCD_QUEUE_BYTES)
                } else {
                    let mut c = [0u8; TX_QUEUE_V0_BYTES]; queue_add_v0(&mut c, ring, counts); p[..TX_QUEUE_V0_BYTES].copy_from_slice(&c);
                    (h::LONG_GROUP, TX_QUEUE_CFG_V0, TX_QUEUE_V0_BYTES)
                }
            }
            Step::Protect => {
                // iwx: nine beacon intervals, or 900 TU without one.
                let tu = if self.target.beacon_interval != 0 { self.target.beacon_interval as u32 * 9 } else { 900 };
                let mut c = [0u8; SESSION_PROTECTION_BYTES]; session_protection(&mut c, ACTION_ADD, tu);
                p[..SESSION_PROTECTION_BYTES].copy_from_slice(&c);
                (h::MAC_CONF_GROUP, h::SESSION_PROTECTION_CMD, SESSION_PROTECTION_BYTES)
            }
            Step::Joined | Step::Failed(_) => (0, 0, 0),
        }
    }

    /// Checks an answer's contents where iwx does (binding, station, queue).
    fn check(&mut self, step: Step, data: &[u8], group: u8, opcode: u8) -> Result<(), Failure> {
        let bad = Failure::BadAnswer { group, opcode, len: data.len() };
        let word = |at: usize| data.get(at..at + 4).map(|w| u32::from_le_bytes(w.try_into().unwrap()));
        match step {
            Step::Binding => { let s = word(0).ok_or(bad)?; self.status = s; if s != 0 { return Err(bad); } }
            Step::Station => { let s = word(0).ok_or(bad)?; self.status = s; if s & 0xff != ADD_STA_SUCCESS { return Err(bad); } }
            Step::Queue => {
                // queue_number, flags, write_pointer, reserved: the queue must be ours and empty.
                if data.len() != 8 { return Err(bad); }
                let q = u16::from_le_bytes([data[0], data[1]]);
                let wp = u16::from_le_bytes([data[4], data[5]]);
                self.status = q as u32 | (wp as u32) << 16;
                if q != MGMT_QUEUE || wp != 0 { return Err(bad); }
            }
            _ => {}
        }
        Ok(())
    }

    /// One step; the caller has already read the receive ring into `cmdq`.
    /// Returns true while more steps are needed.
    pub fn advance<D: Dma + DmaRead>(&mut self, b: &mut Bringup, cmdq: &mut CommandQueue, r: &mut impl Registers, dma: &mut D, now: u64) -> bool {
        if self.done() { return false; }
        let cause = r.read(csr::INT);
        if cause == u32::MAX { self.step = Step::Failed(Failure::Gone); return false; }
        if cause & (csr::INT_SW_ERR | csr::INT_HW_ERR) != 0 { self.step = Step::Failed(Failure::Crashed { cause }); return false; }
        if cause != 0 { r.write(csr::INT, cause); }
        let mut error = None;
        if let Err(id) = b.receive(r, dma, |p: &Packet<'_>| {
            if !cmdq.on_packet(p) { if let Some(e) = h::parse_error(p) { error = Some(e); } }
        }) { self.step = Step::Failed(Failure::BadRxBuffer(id)); return false; }
        if let Some(e) = error { self.step = Step::Failed(Failure::Firmware(e)); return false; }
        let step = self.step;
        if !self.sent {
            let mut p = [0u8; MAC_CONTEXT_BYTES];
            let (group, opcode, len) = self.command(step, b, &mut p);
            match cmdq.send(dma, r, &b.layout, b.base, group, opcode, 0, &p[..len]) {
                Ok(_) => { self.sent = true; self.since = now; }
                Err(_) => self.step = Step::Failed(Failure::Timeout { group, opcode }),
            }
            return !self.done();
        }
        if let Some(a) = cmdq.answer {
            cmdq.answer = None;
            if a.failed { self.step = Step::Failed(Failure::Refused { group: a.sent.group, opcode: a.sent.opcode }); return false; }
            let n = a.len.min(cmdq.response.len());
            let data = cmdq.response;
            if let Err(f) = self.check(step, &data[..n], a.sent.group, a.sent.opcode) { self.step = Step::Failed(f); return false; }
            let next = self.next(step);
            self.go(next, now);
        } else if now.saturating_sub(self.since) >= COMMAND_US {
            let s = cmdq.pending.unwrap_or(cmdq::Sent { group: 0, opcode: 0, index: 0 });
            cmdq.abandon();
            self.step = Step::Failed(Failure::Timeout { group: s.group, opcode: s.opcode });
        }
        !self.done()
    }
}
