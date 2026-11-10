// SPDX-License-Identifier: GPL-3.0-only
//! Stage W5b: authenticating and associating with the AP once `join` has put the
//! chip on its channel, then telling the firmware we are associated, as iwx(4)
//! and net80211 do between `iwx_auth` and `iwx_run`: an open-system authentication
//! frame, an association request (with our RSN element on a WPA2 network), then
//! the station entry updated (`iwx_add_sta_cmd(sc, in, 1)`) and the MAC context
//! modified with the association ID the AP gave (`iwx_mac_ctxt_cmd(.., 1)`).
//! Frames go out on the management queue (`tx`); answers come up as RX MPDUs.
//! Each frame is tried three times, waiting 300 ms for its answer.
//!
//! WPA3 networks authenticate with SAE instead of open-system frames (W5d), so
//! they are refused here for now. WPA2's 4-way handshake follows (`rsna`, W5c).
use crate::bringup::Bringup;
use crate::cmdq::{self, CommandQueue};
use crate::context::{Dma, DmaRead};
use crate::csr::{self, Registers};
use crate::devices::Family;
use crate::hcmd::{self as h, Packet};
use crate::ieee80211::{self as w, Bss, Security};
use crate::init::Failure;
use crate::join::{self, Target, ACTION_MODIFY, ADD_STA_BYTES, MAC_CONTEXT_BYTES};
use crate::scan::parse_mpdu;
use crate::tx::{self, TxQueue};

pub const ATTEMPTS: u8 = 3;
pub const FRAME_US: u64 = 300_000;
const COMMAND_US: u64 = 1_000_000;
const ADD_STA_SUCCESS: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step { Auth, Assoc, Station, Mac, Associated, Refused(Refusal), Failed(Failure) }

/// Why the AP turned us away, beyond the command failures `init::Failure` names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The AP answered with this 802.11 status code.
    Status { frame: &'static str, status: u16 },
    /// No answer after `ATTEMPTS` tries.
    NoAnswer(&'static str),
    /// The AP sent a deauthentication or disassociation with this reason.
    Disconnected(u16),
    /// The firmware could not send the frame (last TX status).
    NotSent { frame: &'static str, status: u16 },
}

/// A frame from the AP that matters here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Heard { Auth { status: u16, transaction: u16, algorithm: u16 }, Assoc { status: u16, aid: u16 }, Gone(u16) }

pub struct Assoc {
    pub step: Step,
    pub target: Target,
    pub bss: Bss,
    me: [u8; 6],
    family: Family,
    pub tx: TxQueue,
    pub aid: u16,
    pub attempts: u8,
    seq: u16,
    since: u64,
    sent: bool,
    /// The station-entry answer, kept for the log.
    pub status: u32,
    /// The last EAPOL frame the AP sent while association was finishing: the
    /// 4-way handshake's message 1 often follows the association response at once.
    pub early: Option<([u8; crate::eapol::MAX_FRAME], usize)>,
}

impl Assoc {
    /// After a finished join to `bss` (`target` is the join's).
    pub fn new(bss: Bss, target: Target, me: [u8; 6], family: Family, now: u64) -> Self {
        let step = match bss.security {
            Security::Open | Security::Wpa2Personal | Security::Wpa3Transition => Step::Auth,
            Security::Wpa3Personal => Step::Failed(Failure::Unsupported("wpa3_sae")),
            Security::Unsupported => Step::Failed(Failure::Unsupported("security")),
        };
        Self { step, target, bss, me, family, tx: TxQueue::new(family), aid: 0, attempts: 0, seq: 0, since: now, sent: false, status: 0, early: None }
    }
    /// Our address, as the frames carry it.
    pub fn me(&self) -> [u8; 6] { self.me }
    pub fn done(&self) -> bool { matches!(self.step, Step::Associated | Step::Refused(_) | Step::Failed(_)) }
    fn go(&mut self, step: Step, now: u64) { self.step = step; self.since = now; self.sent = false; self.attempts = 0; }
    fn refuse(&mut self, r: Refusal) { self.step = Step::Refused(r); }

    /// The frame for `step` into `out`; returns (length, name).
    pub fn frame(&mut self, step: Step, out: &mut [u8; 256]) -> (usize, &'static str) {
        self.seq = (self.seq + 1) & 0xfff;
        match step {
            Step::Auth => (w::auth(out, &self.bss.bssid, &self.me, self.seq, w::AUTH_OPEN, 1, 0, &[]), "auth"),
            _ => {
                let mut rsn = [0u8; 32];
                let n = if self.bss.security == Security::Open { 0 } else { w::own_rsn(false, &mut rsn) };
                let mut bss = self.bss;
                bss.h2e = false; // RSNXE only with SAE (W5d)
                (w::assoc_request(out, &bss, &self.me, self.seq, &rsn[..n]), "assoc")
            }
        }
    }

    /// Keeps a frame if it is from our AP to us.
    fn hear(&self, frame: &[u8]) -> Option<Heard> {
        if frame.len() < w::MGMT_HEADER || frame[4..10] != self.me || frame[16..22] != self.bss.bssid { return None; }
        if let Some(r) = w::parse_disconnect(frame) { return Some(Heard::Gone(r)); }
        if let Some(a) = w::parse_auth(frame) { return Some(Heard::Auth { status: a.status, transaction: a.transaction, algorithm: a.algorithm }); }
        w::parse_assoc_response(frame).map(|a| Heard::Assoc { status: a.status, aid: a.aid })
    }

    /// One step; returns true while more are needed.
    pub fn advance<D: Dma + DmaRead>(&mut self, b: &mut Bringup, cmdq: &mut CommandQueue, r: &mut impl Registers, dma: &mut D, now: u64) -> bool {
        if self.done() { return false; }
        let cause = r.read(csr::INT);
        if cause == u32::MAX { self.step = Step::Failed(Failure::Gone); return false; }
        if cause & (csr::INT_SW_ERR | csr::INT_HW_ERR) != 0 { self.step = Step::Failed(Failure::Crashed { cause }); return false; }
        if cause != 0 { r.write(csr::INT, cause); }
        let (mut error, mut heard, mut tx_status) = (None, None, None);
        let family = self.family;
        let mut freed: [Option<tx::Status>; 1] = [None];
        let mut early = [0u8; crate::eapol::MAX_FRAME];
        let mut early_len = 0;
        {
            let me = &*self;
            let mut scratch = [0u8; 4096];
            if let Err(id) = b.receive(r, dma, |p: &Packet<'_>| {
                if cmdq.on_packet(p) { return; }
                if let Some(s) = tx::parse_response(p) { freed[0] = Some(s); return; }
                if let Some(rx) = parse_mpdu(p, family, &mut scratch) {
                    if let Some(hd) = me.hear(rx.frame) { heard = Some(hd); }
                    if let Some(e) = crate::rsna::eapol_of(rx.frame, &me.me, &me.bss.bssid).filter(|e| e.len() <= crate::eapol::MAX_FRAME) {
                        early[..e.len()].copy_from_slice(e);
                        early_len = e.len();
                    }
                    return;
                }
                if let Some(e) = h::parse_error(p) { error = Some(e); }
            }) { self.step = Step::Failed(Failure::BadRxBuffer(id)); return false; }
        }
        if let Some(s) = freed[0] {
            // Free the ring as the response says (the packet itself is gone by now).
            self.tx.release(dma, &b.layout, s);
            tx_status = Some(s);
        }
        if early_len > 0 { self.early = Some((early, early_len)); }
        if let Some(e) = error { self.step = Step::Failed(Failure::Firmware(e)); return false; }
        if let Some(Heard::Gone(reason)) = heard { self.refuse(Refusal::Disconnected(reason)); return false; }
        match self.step {
            Step::Auth | Step::Assoc => self.frames(b, r, dma, heard, tx_status, now),
            Step::Station | Step::Mac => self.command(b, cmdq, r, dma, now),
            _ => {}
        }
        !self.done()
    }

    fn frames<D: Dma>(&mut self, b: &Bringup, r: &mut impl Registers, dma: &mut D, heard: Option<Heard>, tx_status: Option<tx::Status>, now: u64) {
        let step = self.step;
        let name = if step == Step::Auth { "auth" } else { "assoc" };
        match (step, heard) {
            (Step::Auth, Some(Heard::Auth { status, transaction: 2, algorithm: w::AUTH_OPEN })) => {
                if status != w::STATUS_SUCCESS { self.refuse(Refusal::Status { frame: name, status }); } else { self.go(Step::Assoc, now); }
                return;
            }
            (Step::Assoc, Some(Heard::Assoc { status, aid })) => {
                if status != w::STATUS_SUCCESS { self.refuse(Refusal::Status { frame: name, status }); }
                else { self.aid = aid; self.go(Step::Station, now); }
                return;
            }
            _ => {}
        }
        let failed_tx = tx_status.filter(|s| !s.ok());
        let resend = !self.sent || failed_tx.is_some() || now.saturating_sub(self.since) >= FRAME_US;
        if !resend { return; }
        if self.sent && self.attempts >= ATTEMPTS {
            self.refuse(match failed_tx.or(self.tx.last.filter(|s| !s.ok())) {
                Some(s) => Refusal::NotSent { frame: name, status: s.status },
                None => Refusal::NoAnswer(name),
            });
            return;
        }
        let mut out = [0u8; 256];
        let (n, _) = self.frame(step, &mut out);
        let rate = tx::management_rate(&self.target);
        match self.tx.send(dma, r, &b.layout, b.base, &out[..n], w::MGMT_HEADER, tx::FLAG_CMD_RATE | tx::FLAG_ENCRYPT_DIS, rate) {
            Ok(_) => { self.sent = true; self.since = now; self.attempts += 1; }
            Err(_) => self.refuse(Refusal::NotSent { frame: name, status: 0 }),
        }
    }

    fn command<D: Dma + DmaRead>(&mut self, b: &Bringup, cmdq: &mut CommandQueue, r: &mut impl Registers, dma: &mut D, now: u64) {
        let step = self.step;
        if !self.sent {
            let mut p = [0u8; MAC_CONTEXT_BYTES];
            let (group, opcode, len) = if step == Step::Station {
                let mut c = [0u8; ADD_STA_BYTES];
                join::add_station(&mut c, &self.target.bssid, true);
                p[..ADD_STA_BYTES].copy_from_slice(&c);
                (h::LONG_GROUP, h::ADD_STA, ADD_STA_BYTES)
            } else {
                join::mac_context(&mut p, ACTION_MODIFY, &self.me, &self.target, Some(self.aid));
                (h::LONG_GROUP, h::MAC_CONTEXT_CMD, MAC_CONTEXT_BYTES)
            };
            match cmdq.send(dma, r, &b.layout, b.base, group, opcode, 0, &p[..len]) {
                Ok(_) => { self.sent = true; self.since = now; }
                Err(_) => self.step = Step::Failed(Failure::Timeout { group, opcode }),
            }
            return;
        }
        if let Some(a) = cmdq.answer {
            cmdq.answer = None;
            if a.failed { self.step = Step::Failed(Failure::Refused { group: a.sent.group, opcode: a.sent.opcode }); return; }
            if step == Step::Station {
                let s = cmdq.response[..a.len.min(cmdq::RESPONSE_BYTES)].get(..4).map(|w| u32::from_le_bytes(w.try_into().unwrap()));
                self.status = s.unwrap_or(0);
                if s.map_or(true, |s| s & 0xff != ADD_STA_SUCCESS) {
                    self.step = Step::Failed(Failure::BadAnswer { group: a.sent.group, opcode: a.sent.opcode, len: a.len });
                    return;
                }
                self.go(Step::Mac, now);
            } else {
                self.go(Step::Associated, now);
            }
        } else if now.saturating_sub(self.since) >= COMMAND_US {
            let s = cmdq.pending.unwrap_or(cmdq::Sent { group: 0, opcode: 0, index: 0 });
            cmdq.abandon();
            self.step = Step::Failed(Failure::Timeout { group: s.group, opcode: s.opcode });
        }
    }
}
