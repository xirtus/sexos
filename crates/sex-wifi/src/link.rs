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
//! A deauthentication or disassociation from the AP ends the link.
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
pub enum Down { Disconnected(u16), Failed(Failure) }

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
    /// Last CCMP packet number per key id and TID (non-QoS frames use TID 0).
    pn: [[Option<u64>; 16]; 4],
    pub sent: u64,
    pub received: u64,
    pub dropped: u64,
    pub last_drop: Option<Drop>,
    pub down: Option<Down>,
    rekey: Option<Rekey>,
    /// Key messages answered after the handshake, keys installed since, and the
    /// last key message refused (bad MIC, replay...).
    pub rekey_messages: u32,
    pub rekeyed: u32,
    pub rekey_error: Option<eapol::Error>,
    pub rate_control: RateControl,
    /// The data rate the firmware last reported choosing (rate_n_flags).
    pub rate: u32,
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
        Self { tx, target, me, family, protected, seq: 0x400, pn: [[None; 16]; 4], sent: 0, received: 0, dropped: 0, last_drop: None, down: None,
            rekey: None, rekey_messages: 0, rekeyed: 0, rekey_error: None, rate_control: RateControl::Off, rate: DATA_RATE }
    }
    /// Answers later key messages with the handshake's state.
    pub fn with_rekey(mut self, r: Rekey) -> Self { self.rekey = Some(r); self }
    /// Hands data rates to the firmware once the link runs, with this version of
    /// TLC_MNG_CONFIG_CMD. Nothing changes when no legacy rate is shared.
    pub fn with_rate_control(mut self, version: u8) -> Self {
        if tlc::rates(&self.target) != 0 { self.rate_control = RateControl::Wanted(version); }
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
        let mut out = [0u8; 24 + 8 + MAX_ETHERNET];
        self.seq = (self.seq + 1) & 0xfff;
        let (hdr, total) = w::from_ethernet(ethernet, &self.target.bssid, self.seq, &mut out).ok_or(tx::Error::TooLong)?;
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
    pub fn poll<D: Dma + DmaRead>(&mut self, b: &mut Bringup, cmdq: &mut CommandQueue, r: &mut impl Registers, dma: &mut D,
        now: u64, mut deliver: impl FnMut(&[u8])) -> bool
    {
        if self.down.is_some() { return false; }
        let cause = r.read(csr::INT);
        if cause == u32::MAX { self.down = Some(Down::Failed(Failure::Gone)); return false; }
        if cause & (csr::INT_SW_ERR | csr::INT_HW_ERR) != 0 { self.down = Some(Down::Failed(Failure::Crashed { cause })); return false; }
        if cause != 0 { r.write(csr::INT, cause); }
        let (family, me, bssid, protected) = (self.family, self.me, self.target.bssid, self.protected);
        let (mut freed, mut gone, mut error, mut rate) = (None, None, None, None);
        let (mut received, mut dropped, mut last_drop) = (0u64, 0u64, None);
        let mut pn = self.pn;
        let mut scratch = [0u8; 4096];
        let mut eth = [0u8; 4096];
        let rekeying = self.rekey.is_some();
        let mut key_frame = [0u8; eapol::MAX_FRAME];
        let mut key_len = 0;
        let result = b.receive(r, dma, |p: &Packet<'_>| {
            if cmdq.on_packet(p) { return; }
            if let Some(s) = tx::parse_response(p) { freed = Some(s); return; }
            if let Some(r) = tlc::parse_update(p) { rate = Some(r); return; }
            let Some(rx) = parse_mpdu(p, family, &mut scratch) else {
                if let Some(e) = h::parse_error(p) { error = Some(e); }
                return;
            };
            let f = rx.frame;
            if f.len() < 24 || f[4..10] != me && f[4] & 1 == 0 { return; }
            if f[0] & 0x0c == 0 {
                if f[16..22] == bssid && f[4..10] == me { if let Some(reason) = w::parse_disconnect(f) { gone = Some(reason); } }
                return;
            }
            // Data from our AP only, and not our own broadcasts relayed back.
            if f[0] & 0x0c != 0x08 || f[10..16] != bssid || f[16..22] == me { return; }
            match decap(f, rx.status, protected, &mut eth) {
                Ok((n, tid, number)) => {
                    if let Some((key, number)) = number {
                        if pn[key][tid].is_some_and(|last| number <= last) { dropped += 1; last_drop = Some(Drop::Replay); return; }
                        pn[key][tid] = Some(number);
                    }
                    let key = n > 14 && eth[12..14] == eapol::ETHERTYPE.to_be_bytes() && eth[6..12] == bssid;
                    if key && rekeying && n - 14 <= eapol::MAX_FRAME {
                        key_frame[..n - 14].copy_from_slice(&eth[14..n]);
                        key_len = n - 14;
                    } else if n > 0 && !key { received += 1; deliver(&eth[..n]); }
                }
                // A frame with no payload (null data) is not an error.
                Err(Drop::Malformed) if w::data_header_len(f) == Some(f.len()) => {}
                Err(d) => { dropped += 1; last_drop = Some(d); }
            }
        });
        self.pn = pn;
        self.received += received;
        self.dropped += dropped;
        if last_drop.is_some() { self.last_drop = last_drop; }
        if let Some(s) = freed { self.tx.release(dma, &b.layout, s); }
        if let Err(id) = result { self.down = Some(Down::Failed(Failure::BadRxBuffer(id))); return false; }
        if let Some(e) = error { self.down = Some(Down::Failed(Failure::Firmware(e))); return false; }
        if let Some(reason) = gone { self.down = Some(Down::Disconnected(reason)); return false; }
        if let Some(rate) = rate { self.rate = rate; }
        if key_len > 0 { self.on_key_message(b, r, dma, &key_frame[..key_len]); }
        // One command at a time: rate control waits for keys, keys for rate control.
        if !self.rate_control_step(b, cmdq, r, dma, now) { return true; }
        let installed = match self.rekey.as_mut().map(|k| k.keys.install(b, cmdq, r, dma, now)) {
            Some(Ok(k)) => k,
            Some(Err(f)) => { self.down = Some(Down::Failed(f)); return false; }
            None => None,
        };
        if let Some(k) = installed {
            self.rekeyed += 1;
            let id = match k { Key::Pairwise(_) => 0, Key::Group(id, _) => id as usize & 3, Key::Integrity(..) => return true };
            self.pn[id] = [None; 16];
        }
        true
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

    /// A key message from the AP after the handshake: answered, its keys queued.
    fn on_key_message<D: Dma>(&mut self, b: &Bringup, r: &mut impl Registers, dma: &mut D, frame: &[u8]) {
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
                    // The answer goes before the new keys are in, under the current ones.
                    if self.send(b, r, dma, &eth[..14 + n]).is_err() { self.rekey_error = Some(eapol::Error::Unexpected); }
                }
            }
            Err(e) => self.rekey_error = Some(e),
        }
    }
}
