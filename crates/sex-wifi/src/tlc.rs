// SPDX-License-Identifier: GPL-3.0-only
//! Stage W5g: firmware rate control, as iwx(4)'s `iwx_rs_init` sets it up. The
//! TLC_MNG_CONFIG_CMD (DATA_PATH group) names the rates the station may use with
//! the AP. From then on the firmware picks each data frame's rate itself, from
//! how earlier frames fared, so data frames go without a rate of their own
//! (`iwx_tx_fill_cmd`: no CMD_RATE flag, rate_n_flags 0).
//!
//! A legacy (non-HT) station on a 20 MHz channel has the firmware choose among
//! the CCK and OFDM rates both sides have: up to 54 Mb/s where the fixed rate gave
//! 12. Since X1 an HT, VHT or HE association also names the MCS sets both sides
//! have per spatial stream, the channel width, short guard intervals, LDPC and
//! STBC (`ax::rates`), and the firmware climbs to HE MCS 11 on two streams.
use crate::ax;
use crate::hcmd::{self as h, Packet};
use crate::join::{Target, STATION_ID};

pub const TLC_MNG_CONFIG_CMD: u8 = 0x0f;
pub const TLC_MNG_UPDATE_NOTIF: u8 = 0xf7;
/// `iwx_tlc_config_cmd_v3` and `_v4` are both 28 bytes.
pub const CONFIG_BYTES: usize = 28;
const NOTIF_FLAG_RATE: u32 = 1;

/// The legacy rates both sides have, in the firmware's order (CCK bits 0..4,
/// OFDM bits 4..12). We have every OFDM rate, and CCK on 2.4 GHz.
pub fn rates(t: &Target) -> u16 {
    let ours: u16 = if t.two_ghz() { 0x0fff } else { 0x0ff0 };
    t.rates & ours
}

/// TLC_MNG_CONFIG_CMD for the AP's legacy rates, in the layout the firmware's
/// `version` of the command wants (4; 3 before it).
pub fn config(out: &mut [u8; CONFIG_BYTES], version: u8, t: &Target) {
    out.fill(0);
    let r = ax::rates(&t.mode(), &t.peer, &t.ours);
    out[0] = STATION_ID;
    out[4] = r.width;
    out[5] = r.mode;
    out[6] = r.chains;
    // v4: SGI widths at 7 (v3 has its A-MSDU switch there, left 0).
    if version >= 4 { out[7] = r.sgi; }
    out[8..10].copy_from_slice(&r.flags.to_le_bytes());
    out[10..12].copy_from_slice(&rates(t).to_le_bytes());
    // ht_rates[stream][width]: three widths per stream in v4 (80, 160, 320 MHz),
    // two in v3; max_mpdu_len follows them.
    let per = if version >= 4 { 3 } else { 2 };
    for (s, widths) in r.mcs.iter().enumerate() {
        for (w, &bits) in widths.iter().enumerate() {
            let at = 12 + (s * per + w) * 2;
            out[at..at + 2].copy_from_slice(&bits.to_le_bytes());
        }
    }
    let at = 12 + 2 * per * 2;
    out[at..at + 2].copy_from_slice(&r.max_mpdu.to_le_bytes());
}

/// The rate the firmware switched to, from a TLC update notification.
pub fn parse_update(p: &Packet<'_>) -> Option<u32> {
    if p.code != TLC_MNG_UPDATE_NOTIF || p.group_id() != h::DATA_PATH_GROUP || p.data.len() < 12 { return None; }
    let d = p.data;
    if d[0] != STATION_ID || u32::from_le_bytes(d[4..8].try_into().unwrap()) & NOTIF_FLAG_RATE == 0 { return None; }
    Some(u32::from_le_bytes(d[8..12].try_into().unwrap()))
}
