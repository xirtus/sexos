// SPDX-License-Identifier: GPL-3.0-only
//! Stage W5c: the WPA2 4-way handshake after association, and the keys it gives
//! installed in the firmware, as net80211's EAPOL code and iwx(4)'s `iwx_set_key`
//! do. EAPOL-Key frames from the AP come up as data frames; `eapol::Supplicant`
//! answers them, and the answers go out on the management queue unencrypted, at
//! the management rate and with high priority (`iwx_tx_fill_cmd`). Message 4 is sent
//! before its keys are installed, so the AP can read it.
//!
//! Keys: firmware with SEC_KEY_CMD (DATA_PATH group 0x18, the x17's) gets
//! `iwx_mld_set_sta_key_cmd`'s command; older firmware gets ADD_STA_KEY v2/v3
//! (`iwx_add_sta_key_cmd`). Both with CCMP. The port opens once the pairwise and
//! group keys are both in.
use crate::bringup::Bringup;
use crate::cmdq::{self, CommandQueue};
use crate::context::{Dma, DmaRead};
use crate::csr::{self, Registers};
use crate::devices::Family;
use crate::eapol::{self, Akm, Install, Supplicant};
use crate::hcmd::{self as h, Packet};
use crate::ieee80211 as w;
use crate::init::Failure;
use crate::join::{Target, STATION_ID};
use crate::scan::parse_mpdu;
use crate::tx::{self, TxQueue};

pub const SEC_KEY_CMD: u8 = 0x18;
pub const SEC_KEY_BYTES: usize = 80;
pub const ADD_STA_KEY_BYTES: usize = 76;
/// The whole handshake must finish within this time (the AP gives up sooner).
pub const HANDSHAKE_US: u64 = 10_000_000;
const COMMAND_US: u64 = 1_000_000;
const LLC_EAPOL: [u8; 8] = [0xaa, 0xaa, 0x03, 0, 0, 0, 0x88, 0x8e];

const SEC_KEY_FLAG_CIPHER_CCMP: u32 = 0x02;
const SEC_KEY_FLAG_MFP: u32 = 0x20;
const SEC_KEY_FLAG_MCAST_KEY: u32 = 0x40;
const STA_KEY_FLG_CCM: u16 = 2;
const STA_KEY_FLG_WEP_KEY_MAP: u16 = 1 << 3;
const STA_KEY_FLG_KEYID_POS: u16 = 8;
const STA_KEY_MULTICAST: u16 = 1 << 14;
const STA_KEY_MFP: u16 = 1 << 15;
const ADD_STA_SUCCESS: u32 = 1;

/// The EAPOL frame in a data frame from `bssid` to `me`, if it is one.
pub fn eapol_of<'a>(frame: &'a [u8], me: &[u8; 6], bssid: &[u8; 6]) -> Option<&'a [u8]> {
    let n = w::data_header_len(frame)?;
    let fc = u16::from_le_bytes([frame[0], frame[1]]);
    if fc & w::FC_PROTECTED != 0 || fc & 0x0300 != w::FC_FROM_DS || frame[4..10] != *me || frame[10..16] != *bssid { return None; }
    let body = &frame[n..];
    (body.len() > LLC_EAPOL.len() && body[..8] == LLC_EAPOL).then(|| &body[8..])
}

/// Which key a command installs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key { Pairwise([u8; 16]), Group(u8, [u8; 16]), Integrity(u16, [u8; 16]) }

/// SEC_KEY_CMD v1, action ADD (`iwx_mld_set_sta_key_cmd`).
pub fn sec_key(out: &mut [u8; SEC_KEY_BYTES], key: Key, mfp: bool) {
    out.fill(0);
    let (id, bytes, flags) = match key {
        Key::Pairwise(k) => (0, k, if mfp { SEC_KEY_FLAG_MFP } else { 0 }),
        Key::Group(id, k) => (id as u32, k, SEC_KEY_FLAG_MCAST_KEY),
        Key::Integrity(id, k) => (id as u32, k, SEC_KEY_FLAG_MCAST_KEY | SEC_KEY_FLAG_MFP),
    };
    out[0..4].copy_from_slice(&1u32.to_le_bytes());
    out[4..8].copy_from_slice(&(1u32 << STATION_ID).to_le_bytes());
    out[8..12].copy_from_slice(&id.to_le_bytes());
    out[12..16].copy_from_slice(&(SEC_KEY_FLAG_CIPHER_CCMP | flags).to_le_bytes());
    out[16..32].copy_from_slice(&bytes);
    // TKIP MIC keys, rx_seq and tx_seq stay 0 (a fresh key, as iwx sends it).
}

/// ADD_STA_KEY v2/v3 (`iwx_add_sta_key_cmd`). Integrity keys are not sent this way.
pub fn add_sta_key(out: &mut [u8; ADD_STA_KEY_BYTES], key: Key, mfp: bool) {
    out.fill(0);
    let (id, bytes, offset, extra) = match key {
        Key::Pairwise(k) => (0u16, k, 0u8, if mfp { STA_KEY_MFP } else { 0 }),
        Key::Group(id, k) => (id as u16, k, 1, STA_KEY_MULTICAST),
        Key::Integrity(id, k) => (id, k, 0, 0),
    };
    out[0] = STATION_ID;
    out[1] = offset;
    let flags = STA_KEY_FLG_CCM | STA_KEY_FLG_WEP_KEY_MAP | (id << STA_KEY_FLG_KEYID_POS) & (3 << STA_KEY_FLG_KEYID_POS) | extra;
    out[2..4].copy_from_slice(&flags.to_le_bytes());
    out[4..20].copy_from_slice(&bytes);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step { Handshake, Connected, Refused(Refusal), Failed(Failure) }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// No finished handshake in time: usually a wrong password (the AP finds our
    /// message 2's MIC wrong and never sends message 3).
    Timeout { messages: u8 },
    /// A frame the supplicant rejected for this reason (downgrade, bad MIC...).
    Handshake(eapol::Error),
    /// The AP sent a deauthentication or disassociation with this reason.
    Disconnected(u16),
    NotSent,
}

pub struct Rsna {
    pub step: Step,
    sup: Supplicant,
    pub tx: TxQueue,
    target: Target,
    me: [u8; 6],
    family: Family,
    keys: Keys,
    pub installed: (bool, bool, bool),
    pub messages: u8,
    seq: u16,
    started: u64,
}

/// Keys to put in the firmware, one command at a time, in the order given.
pub struct Keys {
    sec_key_cmd: bool,
    mfp: bool,
    waiting: [Option<Key>; 3],
    in_flight: bool,
    since: u64,
}

impl Keys {
    pub fn new(sec_key_cmd: bool, mfp: bool) -> Self { Self { sec_key_cmd, mfp, waiting: [None; 3], in_flight: false, since: 0 } }
    /// Nothing waiting and no command unanswered.
    pub fn idle(&self) -> bool { !self.in_flight && self.waiting.iter().all(|k| k.is_none()) }

    pub fn queue(&mut self, i: Install) {
        let mut put = |k: Key| { if let Some(slot) = self.waiting.iter_mut().find(|s| s.is_none()) { *slot = Some(k); } };
        if let Some(k) = i.pairwise { put(Key::Pairwise(k)); }
        if let Some(g) = i.group { put(Key::Group(g.id, g.key)); }
        // Integrity keys need SEC_KEY_CMD; older firmware leaves management frames unprotected.
        if let Some(g) = i.igtk.filter(|_| self.sec_key_cmd) { put(Key::Integrity(g.id, g.key)); }
    }

    /// Takes the answer to the last command and sends the next one. Returns the
    /// key the firmware just took, if any.
    pub fn install<D: Dma + DmaRead>(&mut self, b: &Bringup, cmdq: &mut CommandQueue, r: &mut impl Registers, dma: &mut D, now: u64) -> Result<Option<Key>, Failure> {
        let mut done = None;
        if self.in_flight {
            let Some(a) = cmdq.answer else {
                if now.saturating_sub(self.since) >= COMMAND_US {
                    let s = cmdq.pending.unwrap_or(cmdq::Sent { group: 0, opcode: 0, index: 0 });
                    cmdq.abandon();
                    return Err(Failure::Timeout { group: s.group, opcode: s.opcode });
                }
                return Ok(None);
            };
            cmdq.answer = None;
            self.in_flight = false;
            if a.failed { return Err(Failure::Refused { group: a.sent.group, opcode: a.sent.opcode }); }
            if !self.sec_key_cmd {
                let s = cmdq.response[..a.len.min(cmdq::RESPONSE_BYTES)].get(..4).map(|w| u32::from_le_bytes(w.try_into().unwrap()));
                if s.map_or(true, |s| s & 0xff != ADD_STA_SUCCESS) {
                    return Err(Failure::BadAnswer { group: a.sent.group, opcode: a.sent.opcode, len: a.len });
                }
            }
            done = self.waiting[0].take();
            self.waiting.rotate_left(1);
        }
        let Some(key) = self.waiting[0] else { return Ok(done) };
        let mut p = [0u8; SEC_KEY_BYTES];
        let (group, opcode, len) = if self.sec_key_cmd {
            sec_key(&mut p, key, self.mfp);
            (h::DATA_PATH_GROUP, SEC_KEY_CMD, SEC_KEY_BYTES)
        } else {
            let mut c = [0u8; ADD_STA_KEY_BYTES];
            add_sta_key(&mut c, key, self.mfp);
            p[..ADD_STA_KEY_BYTES].copy_from_slice(&c);
            (h::LONG_GROUP, h::ADD_STA_KEY, ADD_STA_KEY_BYTES)
        };
        match cmdq.send(dma, r, &b.layout, b.base, group, opcode, 0, &p[..len]) {
            Ok(_) => { self.in_flight = true; self.since = now; Ok(done) }
            Err(cmdq::Error::Busy) => Ok(done),
            Err(_) => Err(Failure::Timeout { group, opcode }),
        }
    }
}

/// What stays of the handshake once connected: the supplicant answers the AP's
/// later key messages (group rekeying every hour or so, a pairwise rekey), and
/// their keys go into the firmware the same way.
pub struct Rekey { pub sup: Supplicant, pub keys: Keys }

impl Rsna {
    /// After association with a WPA2 network: `own_rsn` is what our association
    /// request carried, `ap_rsn` the AP's element from its beacon. `tx` is the
    /// association's queue (frames may still be in it).
    #[allow(clippy::too_many_arguments)]
    pub fn new(pmk: [u8; 32], snonce: [u8; 32], own_rsn: &[u8], ap_rsn: &[u8], ap: &w::Rsn, target: Target, me: [u8; 6],
        family: Family, tx: TxQueue, sec_key_cmd: bool, now: u64) -> Self
    {
        let sup = Supplicant::new(Akm::Psk, pmk, target.bssid, me, snonce, own_rsn, ap_rsn);
        Self { step: Step::Handshake, sup, tx, target, me, family, keys: Keys::new(sec_key_cmd, ap.mfpc),
            installed: (false, false, false), messages: 0, seq: 0x100, started: now }
    }
    pub fn done(&self) -> bool { !matches!(self.step, Step::Handshake) }
    pub fn target(&self) -> Target { self.target }
    pub fn me(&self) -> [u8; 6] { self.me }
    /// The connection, once `Connected`: data over the handshake's queue, and the
    /// supplicant kept for rekeying.
    pub fn into_link(self) -> crate::link::Link {
        crate::link::Link::new(self.tx, self.target, self.me, self.family, true).with_rekey(Rekey { sup: self.sup, keys: self.keys })
    }

    /// Hands a frame heard before this state existed (message 1 often comes with
    /// the association response).
    pub fn early<D: Dma>(&mut self, b: &Bringup, r: &mut impl Registers, dma: &mut D, eapol_frame: &[u8]) {
        self.on_eapol(b, r, dma, eapol_frame);
    }

    fn on_eapol<D: Dma>(&mut self, b: &Bringup, r: &mut impl Registers, dma: &mut D, frame: &[u8]) {
        self.messages = self.messages.saturating_add(1);
        let mut reply = [0u8; eapol::MAX_FRAME];
        match self.sup.receive(frame, &mut reply) {
            Ok((n, install)) => {
                if n > 0 && !self.send(b, r, dma, &reply[..n]) { self.step = Step::Refused(Refusal::NotSent); return; }
                self.keys.queue(install);
            }
            // A replayed or out-of-order frame is dropped; a forged one ends it.
            Err(eapol::Error::Replay | eapol::Error::Unexpected) => {}
            Err(e) => self.step = Step::Refused(Refusal::Handshake(e)),
        }
    }

    /// An EAPOL frame to the AP as a data frame (`ieee80211_send_eapol_key`).
    fn send<D: Dma>(&mut self, b: &Bringup, r: &mut impl Registers, dma: &mut D, eapol_frame: &[u8]) -> bool {
        let mut eth = [0u8; 14 + eapol::MAX_FRAME];
        eth[..6].copy_from_slice(&self.target.bssid);
        eth[6..12].copy_from_slice(&self.me);
        eth[12..14].copy_from_slice(&eapol::ETHERTYPE.to_be_bytes());
        eth[14..14 + eapol_frame.len()].copy_from_slice(eapol_frame);
        let mut out = [0u8; 24 + 8 + eapol::MAX_FRAME];
        self.seq = (self.seq + 1) & 0xfff;
        let Some((hdr, total)) = w::from_ethernet(&eth[..14 + eapol_frame.len()], &self.target.bssid, self.seq, &mut out) else { return false };
        let flags = tx::FLAG_CMD_RATE | tx::FLAG_ENCRYPT_DIS | tx::FLAG_HIGH_PRI;
        self.tx.send(dma, r, &b.layout, b.base, &out[..total], hdr, flags, tx::management_rate(&self.target)).is_ok()
    }

    /// One step; returns true while more are needed.
    pub fn advance<D: Dma + DmaRead>(&mut self, b: &mut Bringup, cmdq: &mut CommandQueue, r: &mut impl Registers, dma: &mut D, now: u64) -> bool {
        if self.done() { return false; }
        let cause = r.read(csr::INT);
        if cause == u32::MAX { self.step = Step::Failed(Failure::Gone); return false; }
        if cause & (csr::INT_SW_ERR | csr::INT_HW_ERR) != 0 { self.step = Step::Failed(Failure::Crashed { cause }); return false; }
        if cause != 0 { r.write(csr::INT, cause); }
        let (mut error, mut gone, mut freed) = (None, None, None);
        let mut frame = [0u8; eapol::MAX_FRAME];
        let mut frame_len = 0;
        {
            let (family, me, bssid) = (self.family, self.me, self.target.bssid);
            let mut scratch = [0u8; 4096];
            if let Err(id) = b.receive(r, dma, |p: &Packet<'_>| {
                if cmdq.on_packet(p) { return; }
                if let Some(s) = tx::parse_response(p) { freed = Some(s); return; }
                if let Some(rx) = parse_mpdu(p, family, &mut scratch) {
                    let f = rx.frame;
                    if f.len() >= w::MGMT_HEADER && f[4..10] == me && f[16..22] == bssid {
                        if let Some(reason) = w::parse_disconnect(f) { gone = Some(reason); }
                    }
                    if let Some(e) = eapol_of(f, &me, &bssid).filter(|e| e.len() <= eapol::MAX_FRAME) {
                        frame[..e.len()].copy_from_slice(e);
                        frame_len = e.len();
                    }
                    return;
                }
                if let Some(e) = h::parse_error(p) { error = Some(e); }
            }) { self.step = Step::Failed(Failure::BadRxBuffer(id)); return false; }
        }
        if let Some(s) = freed { self.tx.release(dma, &b.layout, s); }
        if let Some(e) = error { self.step = Step::Failed(Failure::Firmware(e)); return false; }
        if let Some(reason) = gone { self.step = Step::Refused(Refusal::Disconnected(reason)); return false; }
        if frame_len > 0 { self.on_eapol(b, r, dma, &frame[..frame_len]); }
        if self.done() { return false; }
        match self.keys.install(b, cmdq, r, dma, now) {
            Ok(Some(Key::Pairwise(_))) => self.installed.0 = true,
            Ok(Some(Key::Group(..))) => self.installed.1 = true,
            Ok(Some(Key::Integrity(..))) => self.installed.2 = true,
            Ok(None) => {}
            Err(f) => { self.step = Step::Failed(f); return false; }
        }
        if self.installed.0 && self.installed.1 && self.keys.idle() {
            self.step = Step::Connected;
        } else if now.saturating_sub(self.started) >= HANDSHAKE_US {
            self.step = Step::Refused(Refusal::Timeout { messages: self.messages });
        }
        !self.done()
    }
}
