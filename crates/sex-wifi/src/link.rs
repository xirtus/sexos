// SPDX-License-Identifier: GPL-3.0-only
//! Stage W5e: the connection as an Ethernet link for sexnet's IP stack. Ethernet
//! frames out become to-DS data frames on the station's queue; data frames from the
//! AP become Ethernet frames again (`ieee80211_encap` / `ieee80211_decap`, with
//! `iwx_tx` and `iwx_rx_mpdu_mq`'s hardware crypto):
//! - On a WPA2 network a frame goes out with the Protected bit and without
//!   ENCRYPT_DIS: the firmware adds the CCMP header and MIC with the installed key.
//! - Frames in were decrypted by the firmware. They are kept only if it says so
//!   (CCM, DEC_DONE and MIC_OK, as `iwx_rx_hwdecrypt` checks) and their packet
//!   number is newer than the last one for that key and TID (`iwx_ccmp_decap`;
//!   pairwise and group keys count apart). The 8-byte CCMP header is removed, and
//!   the Ethernet frame is cut to the length its IP or ARP packet declares, which
//!   drops a MIC some firmware leaves at the end.
//! - Data goes at a fixed legacy rate (OFDM 12 Mb/s), as management frames do, until
//!   firmware rate control (`tlc`) is configured; then the firmware picks each
//!   frame's rate.
//!
//! - EAPOL-Key frames from the AP after the handshake (group rekeying, which APs
//!   do every hour or so, or a pairwise rekey) come protected like other data. They
//!   go to the handshake's supplicant, kept in `Rekey`, not to the IP stack; the
//!   answer goes back protected and the new keys into the firmware. A key's packet
//!   numbers start over once it is in.
//!
//! A deauthentication or disassociation from the AP ends the link, and so does the
//! firmware reporting that the AP's beacons stopped (`iwx_rx_bmiss`; iwx first
//! probes the AP, we leave rejoining to the service, which rescans).
//!
//! Once up, the link also tells the firmware to pass every multicast frame
//! (MCAST_FILTER_CMD with pass_all, as `iwx_allow_mcast` does), so IPv6 neighbour
//! discovery and mDNS arrive and not only broadcasts.
use crate::bringup::Bringup;
use crate::cmdq::CommandQueue;
use crate::context::{Dma, DmaRead};
use crate::csr::{self, Registers};
use crate::devices::Family;
use crate::hcmd::{self as h, Packet};
use crate::ieee80211 as w;
use crate::init::Failure;
use crate::join::Target;
use crate::scan::parse_mpdu;
use crate::eapol;
use crate::rsna::{Key, Rekey};
use crate::tx::{self, TxQueue};
use crate::tlc;
use crate::cmdq;
use crate::ba;

pub const MAX_ETHERNET: usize = 1514;
const CCMP_HEADER: usize = 8;
const STATUS_SEC_MASK: u32 = 7 << 8;
const STATUS_SEC_CCM: u32 = 2 << 8;
const STATUS_MIC_OK: u32 = 1 << 6;
const STATUS_DEC_DONE: u32 = 1 << 11;
const RATE_ANT_A: u32 = 1 << 14;
const RATE_LEGACY_OFDM: u32 = 1 << 8;
/// OFDM rate index 2: 12 Mb/s.
pub const DATA_RATE: u32 = RATE_ANT_A | RATE_LEGACY_OFDM | 2;

/// Firmware rate control: off (fixed rate), its command waiting to go (with the
/// command's version), sent, on, or refused (the fixed rate stays).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateControl { Off, Wanted(u8), Sent(u64), On, Refused }
const COMMAND_US: u64 = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Down { Disconnected(u16), Failed(Failure), BeaconLoss(u32) }

/// A command sent once after the link is up: waiting to go, sent, taken, or refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setup { Wanted, Sent(u64), On, Refused }

/// Consecutive beacons missed before the link counts as lost: about two seconds
/// at the usual 102.4 ms beacon interval (net80211 probes after 7).
pub const BEACON_LOSS: u32 = 20;
pub const MCAST_FILTER_BYTES: usize = 12;

/// MCAST_FILTER_CMD (`iwx_mcast_filter_cmd`): pass all multicast, drop our own.
pub fn mcast_filter(bssid: &[u8; 6]) -> [u8; MCAST_FILTER_BYTES] {
    let mut c = [0u8; MCAST_FILTER_BYTES];
    c[0] = 1; // filter_own
    c[3] = 1; // pass_all
    c[4..10].copy_from_slice(bssid);
    c
}

/// Consecutive beacons missed since the last frame from the AP, from a missed-beacons
/// notification (both versions keep it at byte 4).
pub fn parse_missed_beacons(p: &Packet<'_>) -> Option<u32> {
    if p.is_response() { return None; }
    if !(p.is(h::LEGACY_GROUP, h::MISSED_BEACONS_NOTIFICATION) || p.is(h::MAC_CONF_GROUP, h::MISSED_BEACONS_NOTIF)) { return None; }
    p.data.get(4..8).map(|b| u32::from_le_bytes(b.try_into().unwrap()))
}

/// The firmware saw this frame before (`IWX_RX_MPDU_RES_STATUS_DUPLICATE`).
const STATUS_DUPLICATE: u32 = 1 << 22;
/// How long a block-ack window may sit with holes before they are given up.
pub const BA_STUCK_US: u64 = 250_000;

/// The block-ack session's own command or frame in flight (stage X2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaStep { Idle, Sent(u64, ba::AddbaRequest), Respond(ba::AddbaRequest, u16), Remove, Removing(u64) }

/// The last frame the replay check accepted on a key and TID: its CCMP packet
/// number, 802.11 sequence number and A-MSDU subframe index (0 when not one).
type Accepted = (u64, u16, u8);

/// Takes frames up in order: the replay check, then EAPOL to the supplicant or
/// the frame to the stack.
struct Intake<'a> {
    pn: &'a mut [[Option<Accepted>; 16]; 4],
    bssid: [u8; 6],
    rekeying: bool,
    key_frame: &'a mut [u8; eapol::MAX_FRAME],
    key_len: &'a mut usize,
    received: u64,
    dropped: u64,
    last_drop: Option<Drop>,
    replay: bool,
}
impl Intake<'_> {
    /// `subframe`: the frame's sequence number and A-MSDU subframe index.
    fn take(&mut self, eth: &[u8], tid: usize, number: Option<(usize, u64)>, subframe: (u16, u8), deliver: &mut impl FnMut(&[u8])) {
        let n = eth.len();
        if let Some((key, number)) = number.filter(|_| self.replay) {
            // A-MSDU subframes after the first share the frame's packet number:
            // the same number passes only for a later subframe of the same frame,
            // so a replayed A-MSDU loses every subframe, not just the first.
            let (seq, index) = subframe;
            let replay = self.pn[key][tid].is_some_and(|(last, last_seq, last_index)|
                number < last || (number == last && !(index > last_index && seq == last_seq)));
            if replay {
                self.dropped += 1;
                self.last_drop = Some(Drop::Replay);
                return;
            }
            self.pn[key][tid] = Some((number, seq, index));
        }
        let key = n > 14 && eth[12..14] == eapol::ETHERTYPE.to_be_bytes() && eth[6..12] == self.bssid;
        if key && self.rekeying && n - 14 <= eapol::MAX_FRAME {
            self.key_frame[..n - 14].copy_from_slice(&eth[14..n]);
            *self.key_len = n - 14;
        } else if n > 0 && !key { self.received += 1; deliver(eth); }
    }
    fn take_slot(&mut self, slots: &ba::Slots, i: usize, tid: usize, deliver: &mut impl FnMut(&[u8])) {
        let n = slots.len[i] as usize;
        self.take(&slots.data[i][..n], tid, slots.pn[i], (0, 0), deliver);
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Malformed { pub head: [u8; MALFORMED_HEAD], pub len: u16, pub status: u32, pub subframe: Option<u8> }
pub const MALFORMED_HEAD: usize = 48;

/// Why a frame from the air was not passed up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drop { NotDecrypted, Replay, Malformed }

pub struct Link {
    pub tx: TxQueue,
    target: Target,
    me: [u8; 6],
    family: Family,
    protected: bool,
    seq: u16,
    /// QoS data (stage X1): sequence numbers count per TID.
    qos: bool,
    qos_seq: [u16; 8],
    /// Last CCMP packet number per key id and TID (non-QoS frames use TID 0).
    pn: [[Option<Accepted>; 16]; 4],
    pub sent: u64,
    /// TX responses from the firmware: frames it reports sent, and refused or
    /// unacknowledged ones with the last such status (`tx::Status::status`).
    pub tx_ok: u64,
    pub tx_failed: u64,
    pub tx_last_failure: u16,
    pub received: u64,
    pub dropped: u64,
    pub last_drop: Option<Drop>,
    /// The last frame dropped as malformed (diagnostics): its first bytes as
    /// they came (after the pad), its length, RX status and A-MSDU subframe.
    pub malformed: Option<Malformed>,
    pub malformed_count: u64,
    /// Frames to us the firmware flagged duplicate, and the RX status and reorder
    /// word of the last frame to us (diagnostics).
    pub dup_flagged: u64,
    pub last_unicast: Option<(u32, u32)>,
    /// Field tests: hand frames up without block-ack reordering; skip the replay check.
    pub ba_bypass: bool,
    pub replay_check: bool,
    pub down: Option<Down>,
    rekey: Option<Rekey>,
    /// Key messages answered after the handshake, keys installed since, and the
    /// last key message refused (bad MIC, replay...).
    pub rekey_messages: u32,
    pub rekeyed: u32,
    pub rekey_error: Option<eapol::Error>,
    /// The answer to a key message, until the TX queue takes it (new keys wait for it).
    reply: Option<([u8; 14 + eapol::MAX_FRAME], usize)>,
    pub rate_control: RateControl,
    /// The data rate the firmware last reported choosing (rate_n_flags).
    pub rate: u32,
    /// MCAST_FILTER_CMD after rate control.
    pub multicast: Setup,
    /// The most consecutive beacons the firmware reported missing.
    pub missed_beacons: u32,
    /// Stage X2: the receive block-ack session, the command that makes one
    /// (None: decline; 0: ADD_STA; else RX_BAID_ALLOCATION_CONFIG_CMD's version),
    /// what is in flight, the AP's request waiting, and counters.
    pub ba: Option<ba::Session>,
    ba_command: Option<u8>,
    pub ba_step: BaStep,
    addba: Option<ba::AddbaRequest>,
    ba_moved: u64,
    pub ba_sessions: u32,
    pub ba_declined: u32,
    pub ba_flushes: u32,
}

/// Ethernet frame out of a data frame body from the AP: checks and strips CCMP.
/// Returns the Ethernet length, the TID and the key id with the packet number, if any.
pub fn decap(frame: &[u8], status: u32, protected_link: bool, out: &mut [u8]) -> Result<(usize, usize, Option<(usize, u64)>), Drop> {
    let hdr = w::data_header_len(frame).ok_or(Drop::Malformed)?;
    let fc = u16::from_le_bytes([frame[0], frame[1]]);
    let tid = if fc & 0x0080 != 0 { (frame[24 + if fc & 0x0300 == 0x0300 { 6 } else { 0 }] & 0x0f) as usize } else { 0 };
    let (body, pn) = if fc & w::FC_PROTECTED != 0 {
        if status & STATUS_SEC_MASK != STATUS_SEC_CCM || status & (STATUS_DEC_DONE | STATUS_MIC_OK) != (STATUS_DEC_DONE | STATUS_MIC_OK) {
            return Err(Drop::NotDecrypted);
        }
        let iv = frame.get(hdr..hdr + CCMP_HEADER).ok_or(Drop::Malformed)?;
        if iv[3] & 0x20 == 0 { return Err(Drop::Malformed); } // ExtIV
        let pn = iv[0] as u64 | (iv[1] as u64) << 8 | (iv[4] as u64) << 16 | (iv[5] as u64) << 24 | (iv[6] as u64) << 32 | (iv[7] as u64) << 40;
        (&frame[hdr + CCMP_HEADER..], Some(((iv[3] >> 6) as usize, pn)))
    } else {
        // Once keys are in, unprotected data is only EAPOL (rekeying), handled elsewhere.
        if protected_link { return Err(Drop::NotDecrypted); }
        (&frame[hdr..], None)
    };
    let n = w::to_ethernet(&frame[..hdr], body, out).ok_or(Drop::Malformed)?;
    Ok((declared(&out[..n]).ok_or(Drop::Malformed)?, tid, pn))
}

/// Where the subframes of an unsplit A-MSDU sit in its frame.
#[derive(Clone, Copy)]
struct AmsduParts { head: usize, count: usize, at: [(u16, u16); MAX_SUBFRAMES] }
const MAX_SUBFRAMES: usize = 16;
impl AmsduParts {
    /// Subframe `i` as a frame of its own (header with the A-MSDU bit cleared,
    /// CCMP header, LLC payload) in `out`; its length and DA/SA.
    fn frame(&self, f: &[u8], i: usize, out: &mut [u8; 4096]) -> (usize, [u8; 12]) {
        let (at, len) = (self.at[i].0 as usize, self.at[i].1 as usize);
        out[..self.head].copy_from_slice(&f[..self.head]);
        let qos = 24 + if f[1] & 3 == 3 { 6 } else { 0 };
        out[qos] &= !0x80;
        out[self.head..self.head + len].copy_from_slice(&f[at + 14..at + 14 + len]);
        let mut addrs = [0u8; 12];
        addrs.copy_from_slice(&f[at..at + 12]);
        (self.head + len, addrs)
    }
}

/// An A-MSDU (QoS A-MSDU bit set) whose body is subframes (DA, SA, length, LLC
/// payload, padded to 4 bytes) rather than one LLC/SNAP payload. On the x17 the
/// AP's TCP ACKs to us stopped arriving mid-stream, all dropped as malformed.
fn amsdu_parts(f: &[u8], status: u32) -> Option<AmsduParts> {
    let hdr = w::data_header_len(f)?;
    let fc = u16::from_le_bytes([f[0], f[1]]);
    if fc & 0x0080 == 0 { return None; }
    let qos = 24 + if fc & 0x0300 == 0x0300 { 6 } else { 0 };
    if f[qos] & 0x80 == 0 { return None; }
    let protected = fc & w::FC_PROTECTED != 0 && status & STATUS_SEC_MASK == STATUS_SEC_CCM;
    let head = hdr + if protected { CCMP_HEADER } else { 0 };
    let body = f.get(head..)?;
    // Already one MSDU (the hardware split it and left the bit set).
    if body.len() >= 6 && body[..6] == [0xaa, 0xaa, 0x03, 0x00, 0x00, 0x00] { return None; }
    let mut parts = AmsduParts { head, count: 0, at: [(0, 0); MAX_SUBFRAMES] };
    let mut at = head;
    while at + 14 <= f.len() && parts.count < MAX_SUBFRAMES {
        let len = u16::from_be_bytes([f[at + 12], f[at + 13]]) as usize;
        if len < 8 || at + 14 + len > f.len() { break; }
        parts.at[parts.count] = (at as u16, len as u16);
        parts.count += 1;
        // Each subframe but the last is padded to 4 bytes, counted from the first.
        at = head + ((at + 14 + len - head + 3) & !3);
    }
    (parts.count > 0).then_some(parts)
}

/// The length of an Ethernet frame by what its packet says it holds: IPv4 total
/// length, IPv6 payload length plus its header, ARP's 28 bytes. Anything after
/// (a MIC, padding) is not the packet's. Other types keep their length.
pub fn declared(e: &[u8]) -> Option<usize> {
    if e.len() < 14 { return if e.is_empty() { Some(0) } else { None }; }
    let be = |at: usize| e.get(at..at + 2).map(|b| u16::from_be_bytes([b[0], b[1]]) as usize);
    let need = match be(12)? {
        0x0800 => 14 + be(16)?,
        0x86dd => 14 + 40 + be(18)?,
        0x0806 => 14 + 28,
        0x888e => 14 + 4 + be(16)?,
        _ => return Some(e.len()),
    };
    (need <= e.len()).then_some(need)
}

impl Link {
    /// Over the association's (or handshake's) queue, once connected.
    pub fn new(tx: TxQueue, target: Target, me: [u8; 6], family: Family, protected: bool) -> Self {
        Self { tx, target, me, family, protected, seq: 0x400, qos: target.mode().qos, qos_seq: [0; 8], pn: [[None; 16]; 4], sent: 0, tx_ok: 0, tx_failed: 0, tx_last_failure: 0, received: 0, dropped: 0, last_drop: None, malformed: None, malformed_count: 0, dup_flagged: 0, last_unicast: None, ba_bypass: false, replay_check: true, down: None,
            rekey: None, rekey_messages: 0, rekeyed: 0, rekey_error: None, reply: None, rate_control: RateControl::Off, rate: DATA_RATE,
            multicast: Setup::Wanted, missed_beacons: 0,
            ba: None, ba_command: None, ba_step: BaStep::Idle, addba: None, ba_moved: 0, ba_sessions: 0, ba_declined: 0, ba_flushes: 0 }
    }
    /// Answers later key messages with the handshake's state.
    pub fn with_rekey(mut self, r: Rekey) -> Self { self.rekey = Some(r); self }
    /// Hands data rates to the firmware once the link runs, with this version of
    /// TLC_MNG_CONFIG_CMD. Nothing changes when no legacy rate is shared.
    pub fn with_rate_control(mut self, version: u8) -> Self {
        if tlc::rates(&self.target) != 0 { self.rate_control = RateControl::Wanted(version); }
        self
    }
    /// Accepts block-ack sessions (stage X2) with this firmware command: 0 for
    /// ADD_STA, else RX_BAID_ALLOCATION_CONFIG_CMD's version. Legacy links decline.
    pub fn with_block_ack(mut self, command: u8) -> Self {
        if self.target.mode().ht { self.ba_command = Some(command); }
        self
    }
    pub fn me(&self) -> [u8; 6] { self.me }
    pub fn up(&self) -> bool { self.down.is_none() }
    pub fn protected(&self) -> bool { self.protected }
    /// Room for another frame on the queue.
    pub fn ready(&self) -> bool { self.up() && (self.tx.queued as usize) < crate::context::TX_SLOTS }

    /// Sends one Ethernet II frame to the AP.
    pub fn send<D: Dma>(&mut self, b: &Bringup, r: &mut impl Registers, dma: &mut D, ethernet: &[u8]) -> Result<(), tx::Error> {
        if ethernet.len() < 14 || ethernet.len() > MAX_ETHERNET { return Err(tx::Error::TooLong); }
        let mut out = [0u8; 26 + 8 + MAX_ETHERNET];
        let (hdr, total) = if self.qos {
            // EAPOL goes as voice, as wpa_supplicant and the kernel send it.
            let tid = if ethernet[12..14] == eapol::ETHERTYPE.to_be_bytes() { 7 } else { w::user_priority(ethernet) };
            let seq = &mut self.qos_seq[tid as usize];
            *seq = (*seq + 1) & 0xfff;
            w::from_ethernet_qos(ethernet, &self.target.bssid, *seq, tid, &mut out)
        } else {
            self.seq = (self.seq + 1) & 0xfff;
            w::from_ethernet(ethernet, &self.target.bssid, self.seq, &mut out)
        }.ok_or(tx::Error::TooLong)?;
        // Our own address as the source, whatever the stack wrote.
        out[10..16].copy_from_slice(&self.me);
        let (rate_flag, rate) = if self.rate_control == RateControl::On { (0, 0) } else { (tx::FLAG_CMD_RATE, DATA_RATE) };
        let flags = if self.protected {
            out[1] |= (w::FC_PROTECTED >> 8) as u8;
            rate_flag
        } else { rate_flag | tx::FLAG_ENCRYPT_DIS };
        self.tx.send(dma, r, &b.layout, b.base, &out[..total], hdr, flags, rate)?;
        self.sent += 1;
        Ok(())
    }

    /// Reads the receive ring: every Ethernet frame from the AP goes to `deliver`,
    /// key messages to the supplicant. Returns false once the link is down.
    /// Block-ack requests are declined (see `poll_with`).
    pub fn poll<D: Dma + DmaRead>(&mut self, b: &mut Bringup, cmdq: &mut CommandQueue, r: &mut impl Registers, dma: &mut D,
        now: u64, deliver: impl FnMut(&[u8])) -> bool
    {
        self.poll_with(b, cmdq, r, dma, now, None, deliver)
    }

    /// `poll` with room to reorder one block-ack session's frames (stage X2):
    /// with `slots` and `with_block_ack`, ADDBA requests are accepted.
    #[allow(clippy::too_many_arguments)]
    pub fn poll_with<D: Dma + DmaRead>(&mut self, b: &mut Bringup, cmdq: &mut CommandQueue, r: &mut impl Registers, dma: &mut D,
        now: u64, mut slots: Option<&mut ba::Slots>, mut deliver: impl FnMut(&[u8])) -> bool
    {
        if self.down.is_some() { return false; }
        let cause = r.read(csr::INT);
        if cause == u32::MAX { self.down = Some(Down::Failed(Failure::Gone)); return false; }
        if cause & (csr::INT_SW_ERR | csr::INT_HW_ERR) != 0 { self.down = Some(Down::Failed(Failure::Crashed { cause })); return false; }
        if cause != 0 { r.write(csr::INT, cause); }
        let (family, me, bssid, protected) = (self.family, self.me, self.target.bssid, self.protected);
        let (mut freed, mut gone, mut error, mut rate) = (None, None, None, None);
        let (mut tx_ok, mut tx_failed, mut tx_failure) = (0u64, 0u64, None);
        let mut missed = 0u32;
        let mut pn = self.pn;
        // A key's counters start over when the firmware takes it, in receive order:
        // frames after its answer are under the new key.
        let mut installing = self.rekey.as_ref().and_then(|k| k.keys.in_flight()).and_then(|k| match k {
            Key::Pairwise(_) => Some(0),
            Key::Group(id, _) => Some(id as usize & 3),
            Key::Integrity(..) => None,
        });
        let mut scratch = [0u8; 4096];
        let mut eth = [0u8; 4096];
        let mut key_frame = [0u8; eapol::MAX_FRAME];
        let mut key_len = 0;
        let mut session = if slots.is_some() { self.ba } else { None };
        let mut addba = None;
        let mut delba = None;
        let mut moved = false;
        let (mut malformed, mut malformed_count) = (None, 0u64);
        let (mut dup_flagged, mut last_status) = (0u64, None);
        let ba_bypass = self.ba_bypass;
        let mut intake = Intake { pn: &mut pn, bssid, rekeying: self.rekey.is_some(), key_frame: &mut key_frame, key_len: &mut key_len, received: 0, dropped: 0, last_drop: None, replay: self.replay_check };
        let result = b.receive(r, dma, |p: &Packet<'_>| {
            if cmdq.on_packet(p) {
                if cmdq.answer.is_some_and(|a| !a.failed) { if let Some(id) = installing.take() { intake.pn[id] = [None; 16]; } }
                return;
            }
            if let Some(s) = tx::parse_response(p) {
                if s.ok() { tx_ok += 1; } else { tx_failed += 1; tx_failure = Some(s.status); }
                freed = Some(s);
                return;
            }
            if let Some(r) = tlc::parse_update(p) { rate = Some(r); return; }
            if let Some(n) = parse_missed_beacons(p) { missed = missed.max(n); return; }
            if let Some((baid, nssn, tid)) = ba::parse_release(p) {
                if let (Some(s), Some(sl)) = (session.as_mut(), slots.as_deref_mut()) {
                    if s.baid == baid && tid.is_none_or(|t| t == s.tid) {
                        let tid = s.tid as usize;
                        s.release(nssn, |i| intake.take_slot(sl, i, tid, &mut deliver));
                        moved = true;
                    }
                }
                return;
            }
            let Some(rx) = parse_mpdu(p, family, &mut scratch) else {
                if let Some(e) = h::parse_error(p) { error = Some(e); }
                return;
            };
            let f = rx.frame;
            if f.len() < 24 || f[4..10] != me && f[4] & 1 == 0 { return; }
            if f[0] & 0x0c == 0 {
                if f[16..22] == bssid && f[4..10] == me {
                    if let Some(reason) = w::parse_disconnect(f) { gone = Some(reason); }
                    if let Some(req) = ba::parse_addba_request(f) { addba = Some(req); }
                    if let Some((tid, _)) = ba::parse_delba(f) { delba = Some(tid); }
                }
                return;
            }
            // Data from our AP only, and not our own broadcasts relayed back.
            if f[0] & 0x0c != 0x08 || f[10..16] != bssid || f[16..22] == me { return; }
            // An A-MSDU the hardware did not split: each subframe on its own, as
            // the frame's header and CCMP header before the subframe's LLC payload.
            let parts = amsdu_parts(f, rx.status);
            let count = parts.map_or(1, |p| p.count);
            for part in 0..count {
            let mut sub = [0u8; 4096];
            let (f, index, addrs) = match parts {
                Some(p) => { let (n, a) = p.frame(f, part, &mut sub); (&sub[..n], part as u8, Some(a)) }
                None => (f, rx.subframe.unwrap_or(0), None),
            };
            match decap(f, rx.status, protected, &mut eth) {
                Ok((n, tid, number)) => {
                    // The subframe's own addresses (in an A-MSDU, address 3 is the BSSID).
                    if let Some(a) = addrs { eth[..12].copy_from_slice(&a); }
                    let subframe = (u16::from_le_bytes([f[22], f[23]]) >> 4, index);
                    let qos_unicast = f[0] & 0x80 != 0 && f[4] & 1 == 0;
                    if f[4] & 1 == 0 { last_status = Some((rx.status, rx.reorder)); }
                    let verdict = match (session.as_mut(), qos_unicast) {
                        (Some(s), true) if !ba_bypass => {
                            // A duplicate is a retry of a frame already taken. The bit
                            // alone is not trusted: on newer firmware bits 16..31 of this
                            // word are the A1 hash and filter status, and a set bit 22
                            // there dropped good frames for no reason.
                            let flagged = rx.status & STATUS_DUPLICATE != 0;
                            if flagged { dup_flagged += 1; }
                            s.frame(rx.reorder, tid as u8, flagged && f[1] & 0x08 != 0, false)
                        }
                        _ => ba::Verdict::Outside,
                    };
                    match verdict {
                        ba::Verdict::Outside | ba::Verdict::Pass => intake.take(&eth[..n], tid, number, subframe, &mut deliver),
                        ba::Verdict::Drop => {}
                        ba::Verdict::Hold(i) => {
                            let (s, sl) = (session.as_mut().unwrap(), slots.as_deref_mut().unwrap());
                            sl.data[i][..n].copy_from_slice(&eth[..n]);
                            sl.len[i] = n as u16;
                            sl.pn[i] = number;
                            s.release(ba::Session::nssn(rx.reorder), |i| intake.take_slot(sl, i, tid, &mut deliver));
                            moved = true;
                        }
                    }
                }
                // A frame with no payload (null data) is not an error.
                Err(Drop::Malformed) if w::data_header_len(f) == Some(f.len()) => {}
                Err(d) => {
                    intake.dropped += 1;
                    intake.last_drop = Some(d);
                    if d == Drop::Malformed {
                        let mut head = [0u8; MALFORMED_HEAD];
                        let k = f.len().min(MALFORMED_HEAD);
                        head[..k].copy_from_slice(&f[..k]);
                        malformed = Some(Malformed { head, len: f.len() as u16, status: rx.status, subframe: rx.subframe });
                        malformed_count += 1;
                    }
                }
            }
            }
        });
        self.dup_flagged += dup_flagged;
        if last_status.is_some() { self.last_unicast = last_status; }
        if malformed.is_some() { self.malformed = malformed; }
        self.malformed_count += malformed_count;
        let (received, dropped, last_drop) = (intake.received, intake.dropped, intake.last_drop);
        self.pn = pn;
        self.received += received;
        self.dropped += dropped;
        if last_drop.is_some() { self.last_drop = last_drop; }
        if slots.is_some() { self.ba = session; }
        // A window the firmware stopped moving (its own release timer should
        // fire first): give up on the holes after a while.
        if moved || self.ba.is_none_or(|s| s.stored == 0) { self.ba_moved = now; }
        else if now.saturating_sub(self.ba_moved) >= BA_STUCK_US {
            if let (Some(s), Some(sl)) = (self.ba.as_mut(), slots.as_deref_mut()) {
                let tid = s.tid as usize;
                let mut pn = self.pn;
                let mut intake = Intake { pn: &mut pn, bssid, rekeying: self.rekey.is_some(), key_frame: &mut key_frame, key_len: &mut key_len, received: 0, dropped: 0, last_drop: None, replay: self.replay_check };
                s.flush(|i| intake.take_slot(sl, i, tid, &mut deliver));
                self.received += intake.received;
                self.dropped += intake.dropped;
                self.pn = pn;
                self.ba_flushes += 1;
            }
            self.ba_moved = now;
        }
        if let Some(s) = freed { self.tx.release(dma, &b.layout, s); }
        self.tx_ok += tx_ok;
        self.tx_failed += tx_failed;
        if let Some(f) = tx_failure { self.tx_last_failure = f; }
        if let Err(id) = result { self.down = Some(Down::Failed(Failure::BadRxBuffer(id))); return false; }
        if let Some(e) = error { self.down = Some(Down::Failed(Failure::Firmware(e))); return false; }
        if let Some(reason) = gone { self.down = Some(Down::Disconnected(reason)); return false; }
        self.missed_beacons = self.missed_beacons.max(missed);
        if missed > BEACON_LOSS { self.down = Some(Down::BeaconLoss(missed)); return false; }
        if let Some(rate) = rate { self.rate = rate; }
        if key_len > 0 { self.on_key_message(&key_frame[..key_len]); }
        if let Some(req) = addba { self.addba = Some(req); }
        if let Some(tid) = delba { if self.ba.is_some_and(|s| s.tid == tid) && self.ba_step == BaStep::Idle { self.ba_step = BaStep::Remove; } }
        // One command at a time: rate control waits for keys, keys for rate control.
        if !self.rate_control_step(b, cmdq, r, dma, now) { return true; }
        if !self.multicast_step(b, cmdq, r, dma, now) { return true; }
        // The answer to a key message goes out before its keys go in.
        if let Some((eth, n)) = self.reply {
            if self.send(b, r, dma, &eth[..n]).is_err() { return true; }
            self.reply = None;
        }
        match self.rekey.as_mut().map(|k| k.keys.install(b, cmdq, r, dma, now)) {
            Some(Ok(Some(_))) => self.rekeyed += 1,
            Some(Err(f)) => { self.down = Some(Down::Failed(f)); return false; }
            _ => {}
        }
        if self.rekey.as_ref().is_none_or(|k| k.keys.idle()) {
            let held = slots.as_deref_mut();
            self.block_ack_step(b, cmdq, r, dma, now, held, &mut deliver);
        }
        true
    }

    /// Stage X2: answers ADDBA requests (after the firmware has the session) and
    /// ends sessions the AP deletes. One command or frame per call.
    #[allow(clippy::too_many_arguments)]
    fn block_ack_step<D: Dma + DmaRead>(&mut self, b: &Bringup, cmdq: &mut CommandQueue, r: &mut impl Registers, dma: &mut D, now: u64,
        slots: Option<&mut ba::Slots>, deliver: &mut impl FnMut(&[u8]))
    {
        if !matches!(self.rate_control, RateControl::Off | RateControl::On | RateControl::Refused) { return; }
        if matches!(self.multicast, Setup::Wanted | Setup::Sent(_)) { return; }
        let can = slots.is_some() && self.ba_command.is_some();
        match self.ba_step {
            BaStep::Idle => {
                if let Some(req) = self.addba.take() {
                    // Already ours (a renewed request): accept at once, start over at its SSN.
                    if let Some(s) = self.ba.filter(|s| s.tid == req.tid && can) {
                        self.ba = Some(ba::Session::new(s.tid, s.baid, req.ssn, ba::window_for(&req)));
                        self.ba_step = BaStep::Respond(req, ba::STATUS_SUCCESS);
                    } else if !can || self.ba.is_some() {
                        self.ba_step = BaStep::Respond(req, ba::STATUS_REQUEST_DECLINED);
                    } else if cmdq.pending.is_some() {
                        self.addba = Some(req); // after the command in flight
                        return;
                    } else {
                        let version = self.ba_command.unwrap();
                        let window = ba::window_for(&req);
                        let sent = if version > 0 {
                            let mut c = [0u8; ba::BAID_CONFIG_BYTES];
                            ba::baid_add(&mut c, req.tid, req.ssn, window);
                            cmdq.send(dma, r, &b.layout, b.base, h::DATA_PATH_GROUP, ba::RX_BAID_ALLOCATION_CONFIG_CMD, 0, &c)
                        } else {
                            let mut c = [0u8; crate::join::ADD_STA_BYTES];
                            ba::add_sta_ba(&mut c, req.tid, req.ssn, window, true);
                            cmdq.send(dma, r, &b.layout, b.base, h::LONG_GROUP, h::ADD_STA, 0, &c)
                        };
                        self.ba_step = match sent {
                            Ok(_) => BaStep::Sent(now, req),
                            Err(_) => BaStep::Respond(req, ba::STATUS_REQUEST_DECLINED),
                        };
                        return;
                    }
                }
                self.block_ack_step_respond(b, r, dma);
            }
            BaStep::Sent(since, req) => {
                if let Some(a) = cmdq.answer.take() {
                    let word = cmdq.response[..a.len.min(cmdq::RESPONSE_BYTES)].get(..4).map(|w| u32::from_le_bytes(w.try_into().unwrap()));
                    let baid = if a.failed { None } else if self.ba_command.unwrap_or(0) > 0 {
                        word.map(|w| w as u8).filter(|&b| b < ba::INVALID_BAID)
                    } else { word.and_then(ba::baid_of_add_sta) };
                    match baid {
                        Some(baid) => {
                            self.ba = Some(ba::Session::new(req.tid, baid, req.ssn, ba::window_for(&req)));
                            self.ba_sessions += 1;
                            self.ba_step = BaStep::Respond(req, ba::STATUS_SUCCESS);
                        }
                        None => self.ba_step = BaStep::Respond(req, ba::STATUS_REQUEST_DECLINED),
                    }
                    self.block_ack_step_respond(b, r, dma);
                } else if now.saturating_sub(since) >= COMMAND_US {
                    cmdq.abandon();
                    self.ba_step = BaStep::Respond(req, ba::STATUS_REQUEST_DECLINED);
                }
            }
            BaStep::Respond(..) => self.block_ack_step_respond(b, r, dma),
            BaStep::Remove => {
                if cmdq.pending.is_some() { return; }
                let Some(mut s) = self.ba.take() else { self.ba_step = BaStep::Idle; return };
                // Whatever is held goes up before the session ends.
                if let Some(sl) = slots {
                    let tid = s.tid as usize;
                    let (mut key_frame, mut key_len) = ([0u8; eapol::MAX_FRAME], 0);
                    let mut pn = self.pn;
                    let mut intake = Intake { pn: &mut pn, bssid: self.target.bssid, rekeying: false, key_frame: &mut key_frame, key_len: &mut key_len, received: 0, dropped: 0, last_drop: None, replay: true };
                    s.flush(|i| intake.take_slot(sl, i, tid, deliver));
                    self.received += intake.received;
                    self.pn = pn;
                }
                let version = self.ba_command.unwrap_or(0);
                let _ = if version > 0 {
                    let mut c = [0u8; ba::BAID_CONFIG_BYTES];
                    ba::baid_remove(&mut c, version, s.tid, s.baid);
                    cmdq.send(dma, r, &b.layout, b.base, h::DATA_PATH_GROUP, ba::RX_BAID_ALLOCATION_CONFIG_CMD, 0, &c)
                } else {
                    let mut c = [0u8; crate::join::ADD_STA_BYTES];
                    ba::add_sta_ba(&mut c, s.tid, 0, 0, false);
                    cmdq.send(dma, r, &b.layout, b.base, h::LONG_GROUP, h::ADD_STA, 0, &c)
                };
                self.ba_step = BaStep::Removing(now);
            }
            BaStep::Removing(since) => {
                if cmdq.answer.take().is_some() || now.saturating_sub(since) >= COMMAND_US {
                    if cmdq.pending.is_some() { cmdq.abandon(); }
                    self.ba_step = BaStep::Idle;
                }
            }
        }
    }

    /// Sends the ADDBA response that is waiting, if the queue has room.
    fn block_ack_step_respond<D: Dma>(&mut self, b: &Bringup, r: &mut impl Registers, dma: &mut D) {
        let BaStep::Respond(req, status) = self.ba_step else { return };
        let mut out = [0u8; 64];
        // Management frames share the non-QoS sequence counter.
        self.seq = (self.seq + 1) & 0xfff;
        let n = ba::addba_response(&mut out, &self.target.bssid, &self.me, self.seq, &req, status);
        let rate = tx::management_rate(&self.target);
        if self.tx.send(dma, r, &b.layout, b.base, &out[..n], w::MGMT_HEADER, tx::FLAG_CMD_RATE | tx::FLAG_ENCRYPT_DIS, rate).is_ok() {
            if status != ba::STATUS_SUCCESS { self.ba_declined += 1; }
            self.ba_step = BaStep::Idle;
        }
    }

    /// Sends TLC_MNG_CONFIG_CMD when nothing else is in flight and takes its answer.
    /// Returns false while it is in flight (the command queue is busy).
    fn rate_control_step<D: Dma + DmaRead>(&mut self, b: &Bringup, cmdq: &mut CommandQueue, r: &mut impl Registers, dma: &mut D, now: u64) -> bool {
        match self.rate_control {
            RateControl::Wanted(version) if cmdq.pending.is_none() && self.rekey.as_ref().is_none_or(|k| k.keys.idle()) => {
                let mut c = [0u8; tlc::CONFIG_BYTES];
                tlc::config(&mut c, version, &self.target);
                match cmdq.send(dma, r, &b.layout, b.base, h::DATA_PATH_GROUP, tlc::TLC_MNG_CONFIG_CMD, 0, &c) {
                    Ok(_) => { self.rate_control = RateControl::Sent(now); false }
                    Err(cmdq::Error::Busy) => true,
                    Err(_) => { self.rate_control = RateControl::Refused; true }
                }
            }
            RateControl::Sent(since) => {
                if let Some(a) = cmdq.answer.take() {
                    self.rate_control = if a.failed { RateControl::Refused } else { RateControl::On };
                    true
                } else if now.saturating_sub(since) >= COMMAND_US {
                    cmdq.abandon();
                    self.rate_control = RateControl::Refused;
                    true
                } else { false }
            }
            _ => true,
        }
    }

    /// Sends MCAST_FILTER_CMD once rate control is settled and takes its answer.
    /// Returns false while it is in flight. A refusal leaves broadcasts working.
    fn multicast_step<D: Dma + DmaRead>(&mut self, b: &Bringup, cmdq: &mut CommandQueue, r: &mut impl Registers, dma: &mut D, now: u64) -> bool {
        match self.multicast {
            Setup::Wanted if matches!(self.rate_control, RateControl::Off | RateControl::On | RateControl::Refused)
                && cmdq.pending.is_none() && self.rekey.as_ref().is_none_or(|k| k.keys.idle()) => {
                let c = mcast_filter(&self.target.bssid);
                match cmdq.send(dma, r, &b.layout, b.base, h::LEGACY_GROUP, h::MCAST_FILTER_CMD, 0, &c) {
                    Ok(_) => { self.multicast = Setup::Sent(now); false }
                    Err(cmdq::Error::Busy) => true,
                    Err(_) => { self.multicast = Setup::Refused; true }
                }
            }
            Setup::Sent(since) => {
                if let Some(a) = cmdq.answer.take() {
                    self.multicast = if a.failed { Setup::Refused } else { Setup::On };
                    true
                } else if now.saturating_sub(since) >= COMMAND_US {
                    cmdq.abandon();
                    self.multicast = Setup::Refused;
                    true
                } else { false }
            }
            _ => true,
        }
    }

    /// A key message from the AP after the handshake: answered, its keys queued.
    fn on_key_message(&mut self, frame: &[u8]) {
        let Some(k) = self.rekey.as_mut() else { return };
        self.rekey_messages += 1;
        let mut reply = [0u8; eapol::MAX_FRAME];
        match k.sup.receive(frame, &mut reply) {
            Ok((n, install)) => {
                k.keys.queue(install);
                if n > 0 {
                    let mut eth = [0u8; 14 + eapol::MAX_FRAME];
                    eth[..6].copy_from_slice(&self.target.bssid);
                    eth[6..12].copy_from_slice(&self.me);
                    eth[12..14].copy_from_slice(&eapol::ETHERTYPE.to_be_bytes());
                    eth[14..14 + n].copy_from_slice(&reply[..n]);
                    // Sent under the current keys, before the new ones go in (`poll`
                    // retries it while the TX queue is full).
                    self.reply = Some((eth, 14 + n));
                }
            }
            Err(e) => self.rekey_error = Some(e),
        }
    }
}
