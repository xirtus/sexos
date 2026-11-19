// SPDX-License-Identifier: GPL-3.0-only
//! Stage X1: Wi-Fi 4, 5 and 6 (802.11n HT, 802.11ac VHT, 802.11ax HE).
//!
//! Until X1 the station joined every network as a legacy (802.11a/g) station on
//! a 20 MHz channel: 54 Mb/s at best. Here the access point's HT, VHT and HE
//! elements are read from its beacon, ours go into the association request, and
//! once the AP has accepted them the firmware is told what was agreed:
//! - the PHY context widens to the AP's 40, 80 or 160 MHz channel
//!   (`iwx_phy_ctxt_update` after association, control channel position from the
//!   primary channel's offset to the centre, `iwx_get_vht_ctrl_pos`);
//! - the station entry gets the widths, MIMO and A-MPDU limits
//!   (`iwx_add_sta_cmd`'s HT branch);
//! - the MAC context gets QoS (the AP's WMM EDCA parameters) and HT protection
//!   (`iwx_mac_ctxt_cmd_common`), and the 802.11ax filter bit on an HE network;
//! - on an HE network, STA_HE_CTXT_CMD gives the firmware the AP's BSS colour,
//!   packet-extension thresholds and MU EDCA parameters (iwlwifi's
//!   `iwl_mvm_cfg_he_sta`; the command layouts are from iwlwifi's
//!   `fw/api/mac.h`, which is dual-licensed GPL-2.0 or BSD-3-Clause);
//! - firmware rate control (`tlc`) gets the MCS sets both sides have, per
//!   spatial stream, so it can use up to HE MCS 11 on two streams at 160 MHz
//!   (2.4 Gb/s on the x17's AX210-class Killer AX1675).
//!
//! 2.4 GHz stays at 20 MHz (as most APs run it, and it leaves room for
//! Bluetooth). 6 GHz and 80+80 MHz channels are not handled here (X3).
use crate::ieee80211::elements;

/// HT capabilities element (id 45) of the AP: the fields the firmware needs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ht {
    pub cap: u16,
    /// A-MPDU parameters: max length exponent (bits 0..2), MPDU density (2..5).
    pub ampdu: u8,
    /// Receive MCS bitmask per spatial stream (MCS 0..7 of streams 1..4).
    pub rx_mcs: [u8; 4],
}
/// HT operation element (id 61).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HtOp {
    pub primary: u8,
    /// Secondary channel offset: 0 none, 1 above, 3 below.
    pub sco: u8,
    /// The AP lets stations use 40 MHz.
    pub any_width: bool,
    /// HT protection mode (operation mode bits 0..1).
    pub protection: u8,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Vht { pub cap: u32, pub rx_mcs: u16, pub tx_mcs: u16 }
/// VHT operation element (id 192): channel width 0 (20/40), 1 (80, 160, 80+80),
/// 2 (160, old style), 3 (80+80, old style) and the segment centres.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VhtOp { pub width: u8, pub seg0: u8, pub seg1: u8 }
/// HE capabilities element (extension 35).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct He {
    pub mac: [u8; 6],
    pub phy: [u8; 11],
    pub rx_mcs_80: u16,
    pub tx_mcs_80: u16,
    /// 0xffff (nothing) when the AP has no 160 MHz.
    pub rx_mcs_160: u16,
    pub tx_mcs_160: u16,
    /// PPE thresholds field, when the AP sends one.
    pub ppe: [u8; PPE_MAX],
    pub ppe_len: u8,
}
pub const PPE_MAX: usize = 25;
/// HE operation element (extension 36).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HeOp {
    /// HE operation parameters and BSS colour byte, as one little-endian word.
    pub params: u32,
}
impl HeOp {
    pub fn bss_color(&self) -> u8 { ((self.params >> 24) & 0x3f) as u8 }
    pub fn default_pe(&self) -> u8 { (self.params & 7) as u8 }
    pub fn rts_threshold(&self) -> u16 { ((self.params >> 4) & 0x3ff) as u16 }
}
/// EDCA parameters of one access category: AIFSN, ECWmin, ECWmax, TXOP limit in
/// 32 µs units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Edca { pub aifsn: u8, pub ecw_min: u8, pub ecw_max: u8, pub txop: u16 }
/// The WMM parameter element (Microsoft OUI type 2 subtype 1): QoS info and EDCA
/// per access category, in ACI order BE, BK, VI, VO.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Wmm { pub qos_info: u8, pub ac: [Edca; 4] }
/// MU EDCA parameter set (extension 38): per ACI (BE, BK, VI, VO) the AIFSN,
/// ECWmin/max nibbles and MU EDCA timer, for trigger-based uplink.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MuEdca { pub ac: [(u8, u8, u8); 4] }

/// What the AP said about Wi-Fi 4/5/6 in its beacon.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Peer {
    pub ht: Option<Ht>,
    pub ht_op: Option<HtOp>,
    pub vht: Option<Vht>,
    pub vht_op: Option<VhtOp>,
    pub he: Option<He>,
    pub he_op: Option<HeOp>,
    pub wmm: Option<Wmm>,
    pub mu_edca: Option<MuEdca>,
}

const WMM_OUI: [u8; 3] = [0x00, 0x50, 0xf2];
pub const EXT_HE_CAP: u8 = 35;
pub const EXT_HE_OP: u8 = 36;
pub const EXT_MU_EDCA: u8 = 38;

fn u16_at(b: &[u8], at: usize) -> u16 { u16::from_le_bytes([b[at], b[at + 1]]) }
fn u32_at(b: &[u8], at: usize) -> u32 { u32::from_le_bytes(b[at..at + 4].try_into().unwrap()) }

impl Peer {
    /// Takes one element of a beacon or probe response; false when it is not one of ours.
    pub fn element(&mut self, id: u8, body: &[u8]) -> bool {
        match id {
            45 if body.len() >= 26 => {
                let mut rx_mcs = [0u8; 4];
                rx_mcs.copy_from_slice(&body[3..7]);
                self.ht = Some(Ht { cap: u16_at(body, 0), ampdu: body[2], rx_mcs });
            }
            61 if body.len() >= 22 => self.ht_op = Some(HtOp {
                primary: body[0], sco: body[1] & 3, any_width: body[1] & 4 != 0, protection: body[2] & 3,
            }),
            191 if body.len() >= 12 => self.vht = Some(Vht { cap: u32_at(body, 0), rx_mcs: u16_at(body, 4), tx_mcs: u16_at(body, 8) }),
            192 if body.len() >= 5 => self.vht_op = Some(VhtOp { width: body[0], seg0: body[1], seg1: body[2] }),
            221 if body.len() >= 24 && body[..3] == WMM_OUI && body[3] == 2 && body[4] == 1 && body[5] == 1 => {
                let mut w = Wmm { qos_info: body[6], ac: [Edca::default(); 4] };
                for rec in body[8..24].chunks_exact(4) {
                    let aci = ((rec[0] >> 5) & 3) as usize;
                    w.ac[aci] = Edca { aifsn: rec[0] & 0xf, ecw_min: rec[1] & 0xf, ecw_max: rec[1] >> 4, txop: u16_at(rec, 2) };
                }
                self.wmm = Some(w);
            }
            255 if !body.is_empty() => return self.extension(body[0], &body[1..]),
            _ => return false,
        }
        true
    }

    fn extension(&mut self, ext: u8, b: &[u8]) -> bool {
        match ext {
            EXT_HE_CAP if b.len() >= 6 + 11 + 4 => {
                let mut he = He { mac: [0; 6], phy: [0; 11], rx_mcs_80: u16_at(b, 17), tx_mcs_80: u16_at(b, 19),
                    rx_mcs_160: 0xffff, tx_mcs_160: 0xffff, ppe: [0; PPE_MAX], ppe_len: 0 };
                he.mac.copy_from_slice(&b[..6]);
                he.phy.copy_from_slice(&b[6..17]);
                let mut at = 21;
                if he.phy[0] & HE_PHY0_160 != 0 {
                    if b.len() < at + 4 { return true; }
                    he.rx_mcs_160 = u16_at(b, at);
                    he.tx_mcs_160 = u16_at(b, at + 2);
                    at += 4;
                }
                if he.phy[0] & HE_PHY0_80P80 != 0 { at += 4; }
                if he.phy[6] & HE_PHY6_PPE_PRESENT != 0 && at < b.len() {
                    let n = (b.len() - at).min(PPE_MAX);
                    he.ppe[..n].copy_from_slice(&b[at..at + n]);
                    he.ppe_len = n as u8;
                }
                self.he = Some(he);
            }
            EXT_HE_OP if b.len() >= 6 => self.he_op = Some(HeOp { params: u32_at(b, 0) }),
            EXT_MU_EDCA if b.len() >= 13 => {
                let mut m = MuEdca::default();
                for rec in b[1..13].chunks_exact(3) {
                    let aci = ((rec[0] >> 5) & 3) as usize;
                    m.ac[aci] = (rec[0] & 0xf, rec[1], rec[2]);
                }
                self.mu_edca = Some(m);
            }
            _ => return false,
        }
        true
    }

    /// QoS (WMM) association: every HT AP has it.
    pub fn qos(&self) -> bool { self.wmm.is_some() }
}

// HT capability bits.
pub const HT_LDPC: u16 = 1 << 0;
pub const HT_40: u16 = 1 << 1;
pub const HT_SM_PS_DISABLED: u16 = 3 << 2;
pub const HT_SGI_20: u16 = 1 << 5;
pub const HT_SGI_40: u16 = 1 << 6;
pub const HT_TX_STBC: u16 = 1 << 7;
pub const HT_RX_STBC_1: u16 = 1 << 8;
pub const HT_RX_STBC_MASK: u16 = 3 << 8;
// VHT capability bits.
pub const VHT_WIDTH_MASK: u32 = 3 << 2;
pub const VHT_WIDTH_160: u32 = 1 << 2;
pub const VHT_RXLDPC: u32 = 1 << 4;
pub const VHT_SGI_80: u32 = 1 << 5;
pub const VHT_SGI_160: u32 = 1 << 6;
pub const VHT_TXSTBC: u32 = 1 << 7;
pub const VHT_RXSTBC_1: u32 = 1 << 8;
pub const VHT_RXSTBC_MASK: u32 = 7 << 8;
pub const VHT_SU_BEAMFORMEE: u32 = 1 << 12;
pub const VHT_BEAMFORMEE_STS_SHIFT: u32 = 13;
pub const VHT_AMPDU_SHIFT: u32 = 23;
pub const VHT_AMPDU_MASK: u32 = 7 << 23;
pub const VHT_EXT_NSS_BW_MASK: u32 = 3 << 30;
// HE capability bits.
pub const HE_MAC1_TF_PAD_16US: u8 = 0x08;
pub const HE_MAC2_32BIT_BA: u8 = 0x20;
pub const HE_MAC2_ACK_EN: u8 = 0x80;
pub const HE_PHY0_40_80_5G: u8 = 0x04;
pub const HE_PHY0_160: u8 = 0x08;
pub const HE_PHY0_80P80: u8 = 0x10;
pub const HE_PHY1_DEVICE_CLASS_A: u8 = 0x10;
pub const HE_PHY1_LDPC: u8 = 0x20;
pub const HE_PHY2_STBC_RX_80: u8 = 0x08;
pub const HE_PHY4_SU_BEAMFORMEE: u8 = 0x01;
pub const HE_PHY4_STS_80_4: u8 = 0x0c;
pub const HE_PHY4_STS_160_4: u8 = 0x60;
pub const HE_PHY6_PPE_PRESENT: u8 = 0x80;
pub const HE_PHY9_PADDING_SHIFT: u8 = 6;

/// What our chip and firmware can do, for the elements we send and the commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ours {
    /// Antennas (bit masks: A = 1, B = 2) for transmit and receive.
    pub tx_antennas: u8,
    pub rx_antennas: u8,
    /// The widest channel we take: 160 MHz on AX200 and later, W20 when the PHY
    /// context cannot be widened after association.
    pub max_width: Width,
    /// STA_HE_CTXT_CMD's version (0: HE is not used), and whether the firmware
    /// takes its multiple-BSSID fields (API bit 52, else version 1's size).
    pub he_ctxt: u8,
    pub he_mbssid: bool,
    /// TLC_MNG_CONFIG_CMD's version (3 or 4).
    pub tlc: u8,
}
impl Default for Ours {
    /// A legacy-only station: no HT, VHT or HE elements, as before X1.
    fn default() -> Self { Self { tx_antennas: 0, rx_antennas: 0, max_width: Width::W20, he_ctxt: 0, he_mbssid: false, tlc: 3 } }
}
impl Ours {
    pub fn streams(&self) -> u8 { (self.tx_antennas.count_ones().min(self.rx_antennas.count_ones()) as u8).clamp(1, 2) }
    /// Whether we offer HT at all (a chip with no antenna bits set does not).
    pub fn ht(&self) -> bool { self.tx_antennas != 0 && self.rx_antennas != 0 }
    /// Our VHT/HE MCS map: MCS 0..9 (VHT) or 0..11 (HE) on each of our streams.
    fn mcs_map(&self, top: u16) -> u16 {
        if self.streams() >= 2 { 0xfff0 | top << 2 | top } else { 0xfffc | top }
    }
}

/// The channel the station runs on once associated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Width { W20, W40, W80, W160 }
impl Width {
    /// PHY context channel mode (`IWX_PHY_VHT_CHANNEL_MODE*`), the TLC width and
    /// ADD_STA's FAT_EN value share this numbering.
    pub fn code(self) -> u8 { match self { Width::W20 => 0, Width::W40 => 1, Width::W80 => 2, Width::W160 => 3 } }
    pub fn mhz(self) -> u16 { 20 << self.code() }
}

/// The agreed mode: which generation and channel both sides use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mode { pub ht: bool, pub vht: bool, pub he: bool, pub width: Width, pub ctrl_pos: u8, pub qos: bool }
impl Mode {
    pub const LEGACY: Self = Self { ht: false, vht: false, he: false, width: Width::W20, ctrl_pos: 0, qos: false };
    /// Wi-Fi generation for the log: 4 (HT), 5 (VHT), 6 (HE), else 0 (legacy).
    pub fn generation(&self) -> u8 { if self.he { 6 } else if self.vht { 5 } else if self.ht { 4 } else { 0 } }
}

/// Control channel position (`iwx_get_vht_ctrl_pos`): 1..4 below (0..3) or
/// above (4..7) the centre of the wide channel.
pub fn ctrl_pos(primary: u8, center: u8) -> u8 {
    match primary as i16 - center as i16 {
        -14 => 3, -10 => 2, -6 => 1, -2 => 0, 2 => 4, 6 => 5, 10 => 6, 14 => 7, _ => 0,
    }
}

/// What both sides have on `channel` (`two_ghz`: 2.4 GHz band).
pub fn mode(peer: &Peer, ours: &Ours, channel: u8, two_ghz: bool) -> Mode {
    let mut m = Mode::LEGACY;
    if !ours.ht() || peer.ht.is_none() || !peer.qos() { return m; }
    m.qos = true;
    m.ht = true;
    m.vht = !two_ghz && peer.vht.is_some();
    m.he = ours.he_ctxt != 0 && peer.he.is_some() && peer.he_op.is_some();
    if two_ghz { return m; }
    let ht_op = peer.ht_op.unwrap_or_default();
    let primary = if ht_op.primary != 0 { ht_op.primary } else { channel };
    if m.vht || m.he {
        if let Some(op) = peer.vht_op {
            let (s0, s1) = (op.seg0, op.seg1);
            let gap = (s1 as i16 - s0 as i16).abs();
            let vht_160 = peer.vht.is_some_and(|v| v.cap & (VHT_WIDTH_MASK | VHT_EXT_NSS_BW_MASK) != 0);
            let wide = ours.max_width == Width::W160 && (vht_160 || peer.he.is_some_and(|h| h.phy[0] & HE_PHY0_160 != 0));
            // The 80 MHz segment holding the primary channel: segment 0, except
            // for an old-style 160 MHz channel, where segment 0 is the centre of
            // all 160 MHz. 80+80 and a 160 we do not take run there.
            let center_80 = if op.width == 2 { if primary < s0 { s0.wrapping_sub(8) } else { s0 + 8 } } else { s0 };
            match op.width {
                1 if s1 != 0 && gap == 8 && wide => { m.width = Width::W160; m.ctrl_pos = ctrl_pos(primary, s1); return m; }
                2 if wide => { m.width = Width::W160; m.ctrl_pos = ctrl_pos(primary, s0); return m; }
                1..=3 if s0 != 0 && ours.max_width >= Width::W80 => { m.width = Width::W80; m.ctrl_pos = ctrl_pos(primary, center_80); return m; }
                _ => {}
            }
        }
    }
    let ap_40 = peer.ht.is_some_and(|h| h.cap & HT_40 != 0);
    if ap_40 && ht_op.any_width && (ht_op.sco == 1 || ht_op.sco == 3) && ours.max_width >= Width::W40 {
        m.width = Width::W40;
        m.ctrl_pos = if ht_op.sco == 1 { 0 } else { 4 }; // secondary above: control 1 below the centre
    }
    m
}

/// Our elements for the association request: HT capabilities, VHT capabilities
/// (5 GHz), the HE capabilities extension and the WMM information element, each
/// only when the AP has the same. Returns where the capability elements end (the
/// WMM element, which is vendor-specific, goes last in the frame) and the total.
pub fn own_elements(peer: &Peer, ours: &Ours, two_ghz: bool, out: &mut [u8]) -> (usize, usize) {
    let mode = mode(peer, ours, 0, two_ghz);
    if !mode.ht { return (0, 0); }
    let two = ours.streams() >= 2;
    let mut n = 0;
    // HT capabilities (26 bytes).
    let mut cap = HT_LDPC | HT_SM_PS_DISABLED | HT_SGI_20 | HT_RX_STBC_1;
    if !two_ghz && ours.max_width >= Width::W40 { cap |= HT_40 | HT_SGI_40; }
    if two { cap |= HT_TX_STBC; }
    let mut ht = [0u8; 28];
    ht[0] = 45;
    ht[1] = 26;
    ht[2..4].copy_from_slice(&cap.to_le_bytes());
    ht[4] = 3 | 5 << 2; // 64 KiB A-MPDUs, 4 µs MPDU density (iwlwifi's)
    ht[5] = 0xff;
    if two { ht[6] = 0xff; }
    ht[17] = 1; // TX MCS set defined, same as receive
    out[n..n + 28].copy_from_slice(&ht);
    n += 28;
    if mode.vht {
        let mut cap = VHT_RXLDPC | VHT_SGI_80 | VHT_RXSTBC_1 | VHT_SU_BEAMFORMEE | 3 << VHT_BEAMFORMEE_STS_SHIFT | 7 << VHT_AMPDU_SHIFT;
        if ours.max_width == Width::W160 { cap |= VHT_WIDTH_160 | VHT_SGI_160; }
        if two { cap |= VHT_TXSTBC; }
        let map = ours.mcs_map(2); // MCS 0..9
        let mut v = [0u8; 14];
        v[0] = 191;
        v[1] = 12;
        v[2..6].copy_from_slice(&cap.to_le_bytes());
        v[6..8].copy_from_slice(&map.to_le_bytes());
        v[10..12].copy_from_slice(&map.to_le_bytes());
        out[n..n + 14].copy_from_slice(&v);
        n += 14;
    }
    if mode.he {
        let map = ours.mcs_map(2); // HE: MCS 0..11
        let wide = !two_ghz && ours.max_width == Width::W160;
        let len = 1 + 6 + 11 + 4 + if wide { 4 } else { 0 };
        out[n] = 255;
        out[n + 1] = len as u8;
        out[n + 2] = EXT_HE_CAP;
        let e = &mut out[n + 3..n + 2 + len];
        e.fill(0);
        e[1] = HE_MAC1_TF_PAD_16US;
        let phy = &mut e[6..17];
        phy[0] = if two_ghz || ours.max_width < Width::W80 { 0 } else { HE_PHY0_40_80_5G | if wide { HE_PHY0_160 } else { 0 } };
        phy[1] = HE_PHY1_DEVICE_CLASS_A | HE_PHY1_LDPC;
        phy[2] = HE_PHY2_STBC_RX_80;
        phy[4] = HE_PHY4_SU_BEAMFORMEE | HE_PHY4_STS_80_4 | HE_PHY4_STS_160_4;
        phy[9] = 2 << HE_PHY9_PADDING_SHIFT; // nominal packet padding 16 µs, no PPE thresholds
        e[17..19].copy_from_slice(&map.to_le_bytes());
        e[19..21].copy_from_slice(&map.to_le_bytes());
        if wide {
            e[21..23].copy_from_slice(&map.to_le_bytes());
            e[23..25].copy_from_slice(&map.to_le_bytes());
        }
        n += 2 + len;
    }
    // WMM information element: version 1, no U-APSD.
    out[n..n + 9].copy_from_slice(&[221, 7, 0x00, 0x50, 0xf2, 2, 0, 1, 0]);
    (n, n + 9)
}

// ADD_STA station flags.
pub const STA_FLG_MAX_AGG_SIZE_SHIFT: u32 = 19;
pub const STA_FLG_MAX_AGG_SIZE_MSK: u32 = 0xf << 19;
pub const STA_FLG_AGG_MPDU_DENS_SHIFT: u32 = 23;
pub const STA_FLG_AGG_MPDU_DENS_MSK: u32 = 7 << 23;
pub const STA_FLG_FAT_EN_SHIFT: u32 = 26;
pub const STA_FLG_MIMO_EN_MIMO2: u32 = 1 << 28;
const MAX_AGG_1024K: u32 = 7;

/// ADD_STA's station_flags for the agreed mode (`iwx_add_sta_cmd`), and the extra
/// mask bits they need. Legacy stations get none.
pub fn station_flags(m: &Mode, peer: &Peer, ours: &Ours) -> (u32, u32) {
    if !m.ht { return (0, 0); }
    let mask = STA_FLG_MAX_AGG_SIZE_MSK | STA_FLG_AGG_MPDU_DENS_MSK;
    let mut f = (m.width.code() as u32) << STA_FLG_FAT_EN_SHIFT;
    let ht = peer.ht.unwrap_or_default();
    if ours.streams() >= 2 {
        let two = if m.he { peer.he.is_some_and(|h| (h.rx_mcs_80 >> 2) & 3 != 3) }
            else if m.vht { peer.vht.is_some_and(|v| (v.rx_mcs >> 2) & 3 != 3) }
            else { ht.rx_mcs[1] != 0 };
        if two { f |= STA_FLG_MIMO_EN_MIMO2; }
    }
    let agg = if m.vht { peer.vht.map_or(0, |v| (v.cap & VHT_AMPDU_MASK) >> VHT_AMPDU_SHIFT) } else { (ht.ampdu & 3) as u32 };
    f |= agg.min(MAX_AGG_1024K) << STA_FLG_MAX_AGG_SIZE_SHIFT;
    // MPDU density: 802.11 codes 4..7 (2, 4, 8, 16 µs) map to the same firmware values.
    let density = ((ht.ampdu >> 2) & 7) as u32;
    if density >= 4 { f |= density << STA_FLG_AGG_MPDU_DENS_SHIFT; }
    (f, mask)
}

// MAC context bits.
pub const MAC_PROT_FLG_HT_PROT: u32 = 1 << 23;
pub const MAC_PROT_FLG_FAT_PROT: u32 = 1 << 24;
pub const MAC_QOS_FLG_UPDATE_EDCA: u32 = 1 << 0;
pub const MAC_QOS_FLG_TGN: u32 = 1 << 1;
pub const MAC_FILTER_IN_11AX: u32 = 1 << 14;

/// (protection_flags, qos_flags, extra filter flags) for MAC_CONTEXT_CMD.
pub fn mac_flags(m: &Mode, peer: &Peer) -> (u32, u32, u32) {
    let mut prot = 0;
    let mut qos = if m.qos { MAC_QOS_FLG_UPDATE_EDCA } else { 0 };
    if m.ht {
        let wide = m.width != Width::W20;
        match peer.ht_op.map_or(0, |o| o.protection) {
            1 | 3 => prot |= MAC_PROT_FLG_HT_PROT | MAC_PROT_FLG_FAT_PROT, // non-member, non-HT mixed
            2 if wide => prot |= MAC_PROT_FLG_HT_PROT | MAC_PROT_FLG_FAT_PROT, // 20 MHz stations present
            _ => {}
        }
        qos |= MAC_QOS_FLG_TGN;
    }
    (prot, qos, if m.he { MAC_FILTER_IN_11AX } else { 0 })
}

/// The AP's EDCA parameters for access category `ac` (BE, BK, VI, VO), when it gave them.
pub fn edca(peer: &Peer, ac: usize) -> Option<Edca> { peer.wmm.map(|w| w.ac[ac]) }

// STA_HE_CTXT_CMD.
pub const STA_HE_CTXT_CMD: u8 = 0x07;
pub const HE_CTXT_V1_BYTES: usize = 80;
pub const HE_CTXT_V2_BYTES: usize = 88;
/// Version 3 has room for 320 MHz packet-extension thresholds (four more bytes).
pub const HE_CTXT_V3_BYTES: usize = 92;
const HE_CTXT_32BIT_BA_BITMAP: u32 = 1 << 7;
const HE_CTXT_PACKET_EXT: u32 = 1 << 8;
const HE_CTXT_ACK_ENABLED: u32 = 1 << 11;
const HE_CTXT_MU_EDCA_CW: u32 = 1 << 12;
const HE_CTXT_NIC_NOT_ACK_ENABLED: u32 = 1 << 13;
const PKT_EXT_NONE: u8 = 7;
const PKT_EXT_BPSK: u8 = 0;
const MAX_TID_COUNT: u8 = 8;
/// Firmware access-category index (BK 0, BE 1, VI 2, VO 3) of each ACI (BE, BK, VI, VO).
const ACI_TO_FW: [usize; 4] = [1, 0, 2, 3];

/// Three bits of the PPE thresholds field at bit `pos`, least significant first.
fn ppe_bits(ppe: &[u8], pos: usize) -> u8 {
    let lo = ppe.get(pos / 8).copied().unwrap_or(0) as u16;
    let hi = ppe.get(pos / 8 + 1).copied().unwrap_or(0) as u16;
    (((hi << 8 | lo) >> (pos % 8)) & 7) as u8
}

/// The packet-extension thresholds the firmware wants, [stream][width][low, high]
/// (`iwl_mvm_set_sta_pkt_ext`, four widths in the v1/v2 layout). None when the AP
/// gave neither PPE thresholds nor a usable nominal padding.
pub fn packet_extension(he: &He) -> Option<[[[u8; 2]; 4]; 2]> {
    let mut t = [[[PKT_EXT_NONE; 2]; 4]; 2];
    if he.phy[6] & HE_PHY6_PPE_PRESENT != 0 && he.ppe_len > 0 {
        let ppe = &he.ppe[..he.ppe_len as usize];
        let nss = ((ppe[0] & 7) + 1).min(2) as usize;
        let ru = (ppe[0] >> 3) & 0xf;
        let mut pos = 7;
        for stream in t.iter_mut().take(nss) {
            for (bw, th) in stream.iter_mut().enumerate() {
                if ru & (1 << bw) == 0 { continue; }
                let high = ppe_bits(ppe, pos);
                let low = ppe_bits(ppe, pos + 3);
                pos += 6;
                *th = [low, high];
            }
        }
        return Some(t);
    }
    let (low, high) = match he.phy[9] >> HE_PHY9_PADDING_SHIFT {
        0 => (PKT_EXT_NONE, PKT_EXT_NONE),
        1 => (PKT_EXT_BPSK, PKT_EXT_NONE),
        2 => (PKT_EXT_NONE, PKT_EXT_BPSK),
        _ => return None,
    };
    for stream in t.iter_mut() { for th in stream.iter_mut() { *th = [low, high]; } }
    Some(t)
}

/// STA_HE_CTXT_CMD for the AP (`iwl_mvm_cfg_he_sta`, v1, v2 or v3 layout).
/// Returns its length, or 0 when the AP has no HE.
pub fn he_context(out: &mut [u8; HE_CTXT_V3_BYTES], station: u8, peer: &Peer, ours: &Ours) -> usize {
    out.fill(0);
    let (Some(he), Some(op)) = (peer.he, peer.he_op) else { return 0 };
    // v3's thresholds cover five widths; everything after them moves by 4.
    let v3 = ours.he_ctxt >= 3 && ours.he_mbssid;
    let (widths, d) = if v3 { (5, 4) } else { (4, 0) };
    out[0] = station;
    out[1] = MAX_TID_COUNT;
    let mut flags = HE_CTXT_NIC_NOT_ACK_ENABLED; // we do not offer ACK-enabled aggregates
    if he.mac[2] & HE_MAC2_32BIT_BA != 0 { flags |= HE_CTXT_32BIT_BA_BITMAP; }
    if he.mac[2] & HE_MAC2_ACK_EN != 0 { flags |= HE_CTXT_ACK_ENABLED; }
    // htc_flags at 16 stay 0: we do not offer the HE variant of the HT control field.
    out[24..40 + d].fill(PKT_EXT_NONE);
    if let Some(t) = packet_extension(&he) {
        flags |= HE_CTXT_PACKET_EXT;
        for (s, stream) in t.iter().enumerate() {
            for (bw, th) in stream.iter().enumerate() {
                let at = 24 + (s * widths + bw) * 2;
                out[at] = th[0];
                out[at + 1] = th[1];
            }
        }
    }
    out[40 + d] = op.bss_color();
    out[41 + d] = op.default_pe();
    out[42 + d..44 + d].copy_from_slice(&op.rts_threshold().to_le_bytes());
    if let Some(mu) = peer.mu_edca {
        flags |= HE_CTXT_MU_EDCA_CW;
        for (aci, &(aifsn, ecw, timer)) in mu.ac.iter().enumerate() {
            let at = 48 + d + ACI_TO_FW[aci] * 8;
            out[at..at + 2].copy_from_slice(&((ecw & 0xf) as u16).to_le_bytes());
            out[at + 2..at + 4].copy_from_slice(&((ecw >> 4) as u16).to_le_bytes());
            out[at + 4..at + 6].copy_from_slice(&((aifsn & 0xf) as u16).to_le_bytes());
            out[at + 6..at + 8].copy_from_slice(&(timer as u16).to_le_bytes());
        }
    }
    out[4..8].copy_from_slice(&flags.to_le_bytes());
    // iwlwifi falls back to version 1 when the firmware lacks the MBSSID API.
    if !ours.he_mbssid { HE_CTXT_V1_BYTES } else if v3 { HE_CTXT_V3_BYTES } else { HE_CTXT_V2_BYTES }
}

// TLC.
pub const TLC_MODE_NON_HT: u8 = 0;
pub const TLC_MODE_HT: u8 = 1;
pub const TLC_MODE_VHT: u8 = 2;
pub const TLC_MODE_HE: u8 = 3;
pub const TLC_FLAGS_STBC: u16 = 1 << 0;
pub const TLC_FLAGS_LDPC: u16 = 1 << 1;

/// MCS bits from a 2-bit VHT/HE map entry: VHT 0..7/8/9, HE 0..7/9/11.
fn mcs_bits(entry: u16, he: bool) -> u16 {
    match (entry, he) {
        (3, _) => 0,
        (0, _) => 0x00ff,
        (1, false) => 0x01ff,
        (1, true) | (2, false) => 0x03ff,
        (2, true) => 0x0fff,
        _ => 0,
    }
}

/// What TLC_MNG_CONFIG_CMD needs beyond the legacy rates: mode, chains, SGI
/// widths, flags, and MCS bits [stream][0: up to 80 MHz, 1: 160 MHz], and the
/// largest MPDU (`iwx_rs_init_v4`, iwlwifi's `rs_fw_set_supp_rates`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rates { pub mode: u8, pub width: u8, pub chains: u8, pub sgi: u8, pub flags: u16, pub mcs: [[u16; 2]; 2], pub max_mpdu: u16 }

pub fn rates(m: &Mode, peer: &Peer, ours: &Ours) -> Rates {
    let mut r = Rates { mode: TLC_MODE_NON_HT, width: 0, chains: 1, sgi: 0, flags: 0, mcs: [[0; 2]; 2], max_mpdu: 2312 };
    if !m.ht { return r; }
    let streams = ours.streams() as usize;
    r.chains = if streams >= 2 { 3 } else { 1 };
    r.width = m.width.code();
    let ht = peer.ht.unwrap_or_default();
    if m.he {
        let he = peer.he.unwrap();
        let our = ours.mcs_map(2);
        r.mode = TLC_MODE_HE;
        for s in 0..streams {
            let pick = |theirs: u16| {
                let (a, b) = ((theirs >> (2 * s)) & 3, (our >> (2 * s)) & 3);
                if a == 3 || b == 3 { 0 } else { mcs_bits(a.min(b), true) }
            };
            r.mcs[s][0] = pick(he.rx_mcs_80);
            if m.width == Width::W160 { r.mcs[s][1] = pick(he.rx_mcs_160); }
        }
        if he.phy[1] & HE_PHY1_LDPC != 0 { r.flags |= TLC_FLAGS_LDPC; }
        if streams >= 2 && he.phy[2] & HE_PHY2_STBC_RX_80 != 0 { r.flags |= TLC_FLAGS_STBC; }
        r.max_mpdu = 3895;
        return r;
    }
    if m.vht {
        let v = peer.vht.unwrap();
        r.mode = TLC_MODE_VHT;
        for s in 0..streams {
            let mut bits = mcs_bits((v.rx_mcs >> (2 * s)) & 3, false);
            if m.width == Width::W20 { bits &= !(1 << 9); } // no MCS 9 at 20 MHz
            r.mcs[s][0] = bits;
            if m.width == Width::W160 { r.mcs[s][1] = bits; }
        }
        if v.cap & VHT_RXLDPC != 0 { r.flags |= TLC_FLAGS_LDPC; }
        if streams >= 2 && v.cap & VHT_RXSTBC_MASK != 0 { r.flags |= TLC_FLAGS_STBC; }
        if v.cap & VHT_SGI_80 != 0 { r.sgi |= 1 << 2; }
        if m.width == Width::W160 && v.cap & VHT_SGI_160 != 0 { r.sgi |= 1 << 3; }
        r.max_mpdu = 3895;
    } else {
        r.mode = TLC_MODE_HT;
        r.mcs[0][0] = ht.rx_mcs[0] as u16;
        if streams >= 2 { r.mcs[1][0] = ht.rx_mcs[1] as u16; }
        r.max_mpdu = 3839;
    }
    if ht.cap & HT_LDPC != 0 { r.flags |= TLC_FLAGS_LDPC; }
    if streams >= 2 && ht.cap & HT_RX_STBC_MASK != 0 { r.flags |= TLC_FLAGS_STBC; }
    if ht.cap & HT_SGI_20 != 0 { r.sgi |= 1 << 0; }
    if ht.cap & HT_SGI_40 != 0 { r.sgi |= 1 << 1; }
    r
}

/// Reads the HT/VHT/HE/WMM elements of an information-element list.
pub fn peer_of(ies: &[u8]) -> Peer {
    let mut p = Peer::default();
    for (id, body) in elements(ies) { p.element(id, body); }
    p
}
