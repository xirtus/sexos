// SPDX-License-Identifier: GPL-3.0-only
//! Stage X2: block acknowledgement on receive, the half of 802.11n aggregation
//! that makes downloads fast. A Wi-Fi 4/5/6 AP sends bulk data as A-MPDUs (up to
//! 64 frames in one transmission) once the station agrees to a block-ack session
//! for that TID. Without one, every frame waits for its own ACK.
//!
//! The AP asks with an ADDBA request (action category 3, action 0). The station
//! tells the firmware first, as iwx(4)'s `iwx_sta_rx_agg` does: with
//! RX_BAID_ALLOCATION_CONFIG_CMD (DATA_PATH 0x16) when the firmware has
//! `CAPA_BAID_ML_SUPPORT` (63, the x17's does), else an ADD_STA modify with
//! `add_immediate_ba_tid`. Either answers with the session's BAID. Then the
//! ADDBA response goes out. A DELBA (action 2) ends the session.
//!
//! The hardware splits the A-MPDU but frames can arrive out of order (a lost
//! subframe is retried in a later A-MPDU). Each RX descriptor carries a reorder
//! word: BAID, the frame's sequence number (SN) and the "next sequence number to
//! release" (NSSN) the firmware computed. `Session` is iwx's `iwx_rx_reorder` and
//! `iwx_release_frames`: frames below NSSN pass at once when nothing is held,
//! others wait in a window of slots (the caller's `Slots`) until NSSN passes
//! them. FRAME_RELEASE (0xc3) and BAR_FRAME_RELEASE (0xc2) notifications move
//! NSSN on when the firmware's reorder timer fires or the AP sends a block-ack
//! request. CCMP replay checks run on frames as they are released, in order.
//!
//! One session at a time (the busiest TID, normally 0); another TID's request is
//! declined and that TID's traffic stays unaggregated. A-MSDUs inside A-MPDUs are
//! declined too (one frame per sequence number keeps the window simple).
use crate::hcmd::{self as h, Packet};
use crate::ieee80211 as w;
use crate::join::{ADD_STA_BYTES, MAC_ID, STATION_ID};
use crate::link::MAX_ETHERNET;

pub const FC_ACTION: u16 = 0x00d0;
pub const CATEGORY_BLOCK_ACK: u8 = 3;
pub const ADDBA_REQUEST: u8 = 0;
pub const ADDBA_RESPONSE: u8 = 1;
pub const DELBA: u8 = 2;
pub const STATUS_SUCCESS: u16 = 0;
pub const STATUS_REQUEST_DECLINED: u16 = 37;
/// Our reorder window (the AP may ask for up to 256 on HE; 64 is iwx's).
pub const WINDOW: usize = 64;

pub const RX_BAID_ALLOCATION_CONFIG_CMD: u8 = 0x16;
pub const BAID_CONFIG_BYTES: usize = 16;
pub const BAR_FRAME_RELEASE: u8 = 0xc2;
pub const FRAME_RELEASE: u8 = 0xc3;
pub const CAPA_BAID_ML_SUPPORT: usize = 63;
pub const INVALID_BAID: u8 = 0x7f;
const BAID_ACTION_ADD: u32 = 0;
const BAID_ACTION_REMOVE: u32 = 2;
const STA_MODIFY_ADD_BA_TID: u8 = 1 << 3;
const STA_MODIFY_REMOVE_BA_TID: u8 = 1 << 4;
pub const ADD_STA_BAID_VALID: u32 = 0x8000;
const REORDER_NSSN_MASK: u32 = 0xfff;
const REORDER_SN_SHIFT: u32 = 12;
const REORDER_BAID_SHIFT: u32 = 24;
const REORDER_OLD_SN: u32 = 1 << 31;

/// An AP's ADDBA request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AddbaRequest { pub token: u8, pub tid: u8, pub amsdu: bool, pub window: u16, pub timeout: u16, pub ssn: u16 }

fn action_body(frame: &[u8], action: u8) -> Option<&[u8]> {
    if frame.len() < w::MGMT_HEADER + 2 || u16::from_le_bytes([frame[0], frame[1]]) & 0x00fc != FC_ACTION { return None; }
    let b = &frame[w::MGMT_HEADER..];
    (b[0] == CATEGORY_BLOCK_ACK && b[1] == action).then_some(&b[2..])
}

pub fn parse_addba_request(frame: &[u8]) -> Option<AddbaRequest> {
    let b = action_body(frame, ADDBA_REQUEST)?;
    if b.len() < 7 { return None; }
    let params = u16::from_le_bytes([b[1], b[2]]);
    Some(AddbaRequest {
        token: b[0], amsdu: params & 1 != 0, tid: ((params >> 2) & 0xf) as u8, window: params >> 6,
        timeout: u16::from_le_bytes([b[3], b[4]]), ssn: u16::from_le_bytes([b[5], b[6]]) >> 4,
    })
}

/// A DELBA: the TID and whether the sender started the session (the AP, for ours).
pub fn parse_delba(frame: &[u8]) -> Option<(u8, bool)> {
    let b = action_body(frame, DELBA)?;
    if b.len() < 4 { return None; }
    let params = u16::from_le_bytes([b[0], b[1]]);
    Some(((params >> 12) as u8 & 0xf, params & (1 << 11) != 0))
}

/// The window we answer with: the AP's (0 means "any"), at most ours.
pub fn window_for(req: &AddbaRequest) -> u16 { if req.window == 0 { WINDOW as u16 } else { req.window.min(WINDOW as u16) } }

/// The ADDBA response (no A-MSDU in A-MPDU, immediate policy, our window).
pub fn addba_response(out: &mut [u8], bssid: &[u8; 6], me: &[u8; 6], seq: u16, req: &AddbaRequest, status: u16) -> usize {
    out[..2].copy_from_slice(&FC_ACTION.to_le_bytes());
    out[2..4].fill(0);
    out[4..10].copy_from_slice(bssid);
    out[10..16].copy_from_slice(me);
    out[16..22].copy_from_slice(bssid);
    out[22..24].copy_from_slice(&(seq << 4).to_le_bytes());
    let params: u16 = 1 << 1 | (req.tid as u16 & 0xf) << 2 | window_for(req) << 6;
    let b = &mut out[24..33];
    b[0] = CATEGORY_BLOCK_ACK;
    b[1] = ADDBA_RESPONSE;
    b[2] = req.token;
    b[3..5].copy_from_slice(&status.to_le_bytes());
    b[5..7].copy_from_slice(&params.to_le_bytes());
    b[7..9].copy_from_slice(&req.timeout.to_le_bytes());
    33
}

/// RX_BAID_ALLOCATION_CONFIG_CMD v2: add a session for our station's `tid`.
pub fn baid_add(out: &mut [u8; BAID_CONFIG_BYTES], tid: u8, ssn: u16, window: u16) {
    out.fill(0);
    out[0..4].copy_from_slice(&BAID_ACTION_ADD.to_le_bytes());
    out[4..8].copy_from_slice(&(1u32 << STATION_ID).to_le_bytes());
    out[8] = tid;
    out[12..14].copy_from_slice(&ssn.to_le_bytes());
    out[14..16].copy_from_slice(&window.to_le_bytes());
}
/// The same command removing it: by BAID in version 1, by station and TID after.
pub fn baid_remove(out: &mut [u8; BAID_CONFIG_BYTES], version: u8, tid: u8, baid: u8) {
    out.fill(0);
    out[0..4].copy_from_slice(&BAID_ACTION_REMOVE.to_le_bytes());
    if version == 1 {
        out[4..8].copy_from_slice(&(baid as u32).to_le_bytes());
    } else {
        out[4..8].copy_from_slice(&(1u32 << STATION_ID).to_le_bytes());
        out[8..12].copy_from_slice(&(tid as u32).to_le_bytes());
    }
}
/// ADD_STA modify adding (`start`) or removing a receive session (older firmware).
pub fn add_sta_ba(out: &mut [u8; ADD_STA_BYTES], tid: u8, ssn: u16, window: u16, start: bool) {
    out.fill(0);
    out[0] = 1; // modify
    out[4..8].copy_from_slice(&MAC_ID.to_le_bytes());
    out[16] = STATION_ID;
    if start {
        out[17] = STA_MODIFY_ADD_BA_TID;
        out[28] = tid;
        out[30..32].copy_from_slice(&ssn.to_le_bytes());
        out[44..46].copy_from_slice(&window.to_le_bytes());
    } else {
        out[17] = STA_MODIFY_REMOVE_BA_TID;
        out[29] = tid;
    }
}
/// The BAID in an ADD_STA answer's status word.
pub fn baid_of_add_sta(status: u32) -> Option<u8> {
    (status & 0xff == 1 && status & ADD_STA_BAID_VALID != 0).then_some(((status >> 8) & 0x7f) as u8).filter(|&b| b != INVALID_BAID)
}

/// A firmware release notification: (BAID, NSSN), and for BAR releases the TID.
pub fn parse_release(p: &Packet<'_>) -> Option<(u8, u16, Option<u8>)> {
    if p.is_response() { return None; }
    if p.is(h::LEGACY_GROUP, FRAME_RELEASE) && p.data.len() >= 4 {
        return Some((p.data[0], u16::from_le_bytes([p.data[2], p.data[3]]) & 0xfff, None));
    }
    if p.is(h::LEGACY_GROUP, BAR_FRAME_RELEASE) && p.data.len() >= 8 {
        let sta_tid = u32::from_le_bytes(p.data[0..4].try_into().unwrap());
        let info = u32::from_le_bytes(p.data[4..8].try_into().unwrap());
        return Some((((info >> 24) & 0x3f) as u8, (info & 0xfff) as u16, Some((sta_tid & 0xf) as u8)));
    }
    None
}

/// Sequence number `a` before `b`, modulo 4096.
pub fn seq_lt(a: u16, b: u16) -> bool { a != b && (b.wrapping_sub(a) & 0xfff) < 0x800 }

/// What to do with a frame of the session's TID.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Not part of the session (other TID, multicast, no BAID): as before.
    Outside,
    /// In order: hand it up now.
    Pass,
    /// Held in slot `n` until NSSN passes it; then `release` with the NSSN.
    Hold(usize),
    /// Duplicate, below the window, or a block-ack request (whose NSSN to release).
    Drop,
}

/// One receive block-ack session and its reorder window (`iwx_reorder_buffer`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Session {
    pub tid: u8,
    pub baid: u8,
    pub window: u16,
    head: u16,
    held: u64,
    valid: bool,
    /// Frames held now, passed straight through, released from the window, dropped.
    pub stored: u16,
    pub passed: u64,
    pub released: u64,
    pub dropped: u64,
    /// Frames dropped in a row (old or duplicate): a firmware window out of step
    /// with the AP drops every frame, and the link gives up on reordering.
    pub consecutive_drops: u32,
}

impl Session {
    pub fn new(tid: u8, baid: u8, ssn: u16, window: u16) -> Self {
        Self { tid, baid, window: window.clamp(1, WINDOW as u16), head: ssn & 0xfff, held: 0, valid: false, stored: 0, passed: 0, released: 0, dropped: 0, consecutive_drops: 0 }
    }
    pub fn head(&self) -> u16 { self.head }
    fn slot(&self, sn: u16) -> usize { (sn % self.window) as usize }

    /// A unicast QoS data frame (or block-ack request, `bar`) with this
    /// descriptor `reorder` word, `tid`, and the firmware's duplicate bit.
    /// `amsdu`: for one subframe of an A-MSDU, its index and whether it is the
    /// last. Subframes share the sequence number; the head moves on the last one
    /// only (`iwl_mvm_reorder`).
    pub fn frame(&mut self, reorder: u32, tid: u8, duplicate: bool, bar: bool, amsdu: Option<(u8, bool)>) -> Verdict {
        let (index, last) = amsdu.unwrap_or((0, true));
        let baid = ((reorder >> REORDER_BAID_SHIFT) & 0x7f) as u8;
        if baid == INVALID_BAID || baid != self.baid || tid != self.tid { return Verdict::Outside; }
        let nssn = (reorder & REORDER_NSSN_MASK) as u16;
        let sn = ((reorder >> REORDER_SN_SHIFT) & 0xfff) as u16;
        let old = reorder & REORDER_OLD_SN != 0;
        if !self.valid {
            if old { return Verdict::Outside; }
            self.valid = true;
        }
        if bar { return Verdict::Drop; } // the caller releases up to NSSN
        if duplicate || old { self.dropped += 1; self.consecutive_drops += 1; return Verdict::Drop; }
        self.consecutive_drops = 0;
        if self.stored == 0 && seq_lt(sn, nssn) {
            if last && seq_lt(self.head, nssn) { self.head = nssn; }
            self.passed += 1;
            return Verdict::Pass;
        }
        if self.stored == 0 && sn == self.head {
            if last { self.head = (self.head + 1) & 0xfff; }
            self.passed += 1;
            return Verdict::Pass;
        }
        // A later subframe of an A-MSDU whose first one already went up (or is
        // held): up now, rather than lost (one slot holds one frame).
        if index > 0 && (seq_lt(sn, self.head) || self.held & (1 << self.slot(sn)) != 0) { self.passed += 1; return Verdict::Pass; }
        // Below the window: already released or given up on.
        if seq_lt(sn, self.head) { self.dropped += 1; return Verdict::Drop; }
        let slot = self.slot(sn);
        if self.held & (1 << slot) != 0 { self.dropped += 1; return Verdict::Drop; }
        self.held |= 1 << slot;
        self.stored += 1;
        Verdict::Hold(slot)
    }

    /// NSSN of a reorder word.
    pub fn nssn(reorder: u32) -> u16 { (reorder & REORDER_NSSN_MASK) as u16 }

    /// Releases held frames below `nssn`, in order, by slot.
    pub fn release(&mut self, nssn: u16, mut out: impl FnMut(usize)) {
        while seq_lt(self.head, nssn) {
            let slot = self.slot(self.head);
            if self.held & (1 << slot) != 0 {
                self.held &= !(1 << slot);
                self.stored -= 1;
                self.released += 1;
                out(slot);
            }
            self.head = (self.head + 1) & 0xfff;
        }
    }

    /// Gives up on the holes: releases everything held, in order (a stuck window).
    pub fn flush(&mut self, out: impl FnMut(usize)) {
        if self.stored == 0 { return; }
        let mut last = self.head;
        let mut sn = self.head;
        for _ in 0..self.window {
            if self.held & (1 << self.slot(sn)) != 0 { last = sn; }
            sn = (sn + 1) & 0xfff;
        }
        self.release((last + 1) & 0xfff, out);
    }
}

/// Room for the frames a session holds: Ethernet frames after decryption, with
/// the key id and packet number their replay check needs at release.
pub struct Slots {
    pub len: [u16; WINDOW],
    pub pn: [Option<(usize, u64)>; WINDOW],
    /// Sequence number and A-MSDU subframe index, for the replay check at release.
    pub sub: [(u16, u8); WINDOW],
    pub data: [[u8; MAX_ETHERNET]; WINDOW],
}
impl Slots {
    pub const EMPTY: Self = Self { len: [0; WINDOW], pn: [None; WINDOW], sub: [(0, 0); WINDOW], data: [[0; MAX_ETHERNET]; WINDOW] };
}
