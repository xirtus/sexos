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
//! WPA3 networks authenticate with SAE instead (W5d, `Assoc::with_sae`): a commit
//! (transaction 1) and a confirm (transaction 2) each way, as net80211 and
//! wpa_supplicant's `sae.c` exchange them. The commit carries status 126 when the
//! AP does hash-to-element; an AP under load asks for an anti-clogging token first
//! (status 76) and the commit is sent again with it. The PMK SAE gives then feeds
//! the 4-way handshake (`rsna`, W5c), as WPA2's passphrase hash does. WPA3
//! requires management frame protection, which the RSN element says.
//! Transition networks (WPA2 and WPA3) keep using WPA2.
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
use crate::sae::{self, Sae};

pub const ATTEMPTS: u8 = 3;
pub const FRAME_US: u64 = 300_000;
const COMMAND_US: u64 = 1_000_000;
const ADD_STA_SUCCESS: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step { SaeCommit, SaeConfirm, Auth, Assoc, Station, Mac, Associated, Refused(Refusal), Failed(Failure) }

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
    /// The AP's SAE commit or confirm did not check out (a wrong password shows
    /// as a bad confirm).
    Sae(sae::Error),
}

/// Status codes of SAE authentication frames.
pub const STATUS_ANTI_CLOGGING_TOKEN: u16 = 76;
pub const STATUS_SAE_HASH_TO_ELEMENT: u16 = 126;
pub const AUTH_SAE: u16 = 3;
const TOKEN_BYTES: usize = 64;

/// Our side of SAE while it runs.
struct SaeRun { sae: Sae, h2e: bool, token: [u8; TOKEN_BYTES], token_len: usize }

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
    sae: Option<SaeRun>,
}

impl Assoc {
    /// After a finished join to `bss` (`target` is the join's).
    pub fn new(bss: Bss, target: Target, me: [u8; 6], family: Family, now: u64) -> Self {
        let step = match bss.security {
            Security::Open | Security::Wpa2Personal | Security::Wpa3Transition => Step::Auth,
            Security::Wpa3Personal => Step::Failed(Failure::Unsupported("wpa3_sae")),
            Security::Unsupported => Step::Failed(Failure::Unsupported("security")),
        };
        Self { step, target, bss, me, family, tx: TxQueue::new(family), aid: 0, attempts: 0, seq: 0, since: now, sent: false, status: 0, early: None, sae: None }
    }
    /// After a finished join to a WPA3 network: SAE with `sae`, built on the
    /// password element for this AP and us (hash-to-element when `h2e`).
    pub fn with_sae(bss: Bss, target: Target, me: [u8; 6], family: Family, now: u64, sae: Sae, h2e: bool) -> Self {
        let mut a = Self::new(bss, target, me, family, now);
        a.step = Step::SaeCommit;
        a.bss.h2e = h2e;
        a.sae = Some(SaeRun { sae, h2e, token: [0; TOKEN_BYTES], token_len: 0 });
        a
    }
    /// The PMK SAE agreed on, once confirmed.
    pub fn sae_pmk(&self) -> Option<[u8; 32]> {
        self.sae.as_ref().filter(|_| matches!(self.step, Step::Assoc | Step::Station | Step::Mac | Step::Associated)).map(|s| s.sae.pmk)
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
            Step::SaeCommit => {
                let s = self.sae.as_ref().unwrap();
                let commit = s.sae.commit();
                // Group, then the token (inside its container element with H2E), scalar, element.
                let mut body = [0u8; sae::COMMIT_LEN + TOKEN_BYTES + 3];
                let mut n = 0;
                body[..2].copy_from_slice(&commit[..2]);
                n += 2;
                if !s.h2e { body[n..n + s.token_len].copy_from_slice(&s.token[..s.token_len]); n += s.token_len; }
                body[n..n + sae::COMMIT_LEN - 2].copy_from_slice(&commit[2..]);
                n += sae::COMMIT_LEN - 2;
                if s.h2e && s.token_len > 0 {
                    body[n..n + 3].copy_from_slice(&[255, s.token_len as u8 + 1, 93]);
                    body[n + 3..n + 3 + s.token_len].copy_from_slice(&s.token[..s.token_len]);
                    n += 3 + s.token_len;
                }
                let status = if s.h2e { STATUS_SAE_HASH_TO_ELEMENT } else { 0 };
                (w::auth(out, &self.bss.bssid, &self.me, self.seq, AUTH_SAE, 1, status, &body[..n]), "sae_commit")
            }
            Step::SaeConfirm => {
                let c = self.sae.as_ref().unwrap().sae.confirm(1);
                (w::auth(out, &self.bss.bssid, &self.me, self.seq, AUTH_SAE, 2, 0, &c), "sae_confirm")
            }
            _ => {
                let mut rsn = [0u8; 32];
                let n = if self.bss.security == Security::Open { 0 } else { w::own_rsn(self.sae.is_some(), &mut rsn) };
                let mut bss = self.bss;
                bss.h2e &= self.sae.as_ref().is_some_and(|s| s.h2e); // RSNXE only with SAE
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
        let mut auth_body = [0u8; 256];
        let mut auth_len = 0;
        {
            let me = &*self;
            let mut scratch = [0u8; 4096];
            if let Err(id) = b.receive(r, dma, |p: &Packet<'_>| {
                if cmdq.on_packet(p) { return; }
                if let Some(s) = tx::parse_response(p) { freed[0] = Some(s); return; }
                if let Some(rx) = parse_mpdu(p, family, &mut scratch) {
                    if let Some(hd) = me.hear(rx.frame) {
                        heard = Some(hd);
                        if let Some(a) = w::parse_auth(rx.frame).filter(|a| a.algorithm == AUTH_SAE && a.body.len() <= 256) {
                            auth_body[..a.body.len()].copy_from_slice(a.body);
                            auth_len = a.body.len();
                        }
                    }
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
            Step::SaeCommit | Step::SaeConfirm | Step::Auth | Step::Assoc => self.frames(b, r, dma, heard, &auth_body[..auth_len], tx_status, now),
            Step::Station | Step::Mac => self.command(b, cmdq, r, dma, now),
            _ => {}
        }
        !self.done()
    }

    fn frames<D: Dma>(&mut self, b: &Bringup, r: &mut impl Registers, dma: &mut D, heard: Option<Heard>, body: &[u8], tx_status: Option<tx::Status>, now: u64) {
        let step = self.step;
        let name = match step { Step::Auth => "auth", Step::SaeCommit => "sae_commit", Step::SaeConfirm => "sae_confirm", _ => "assoc" };
        match (step, heard) {
            (Step::SaeCommit, Some(Heard::Auth { status, transaction: 1, algorithm: AUTH_SAE })) => {
                let s = self.sae.as_mut().unwrap();
                if status == STATUS_ANTI_CLOGGING_TOKEN {
                    // Group, then the token (with H2E inside its container element).
                    let token = body.get(2..).map(|t| if s.h2e && t.len() >= 3 && t[0] == 255 && t[2] == 93 { &t[3..] } else { t }).unwrap_or(&[]);
                    if token.is_empty() || token.len() > TOKEN_BYTES { self.refuse(Refusal::Status { frame: name, status }); return; }
                    s.token[..token.len()].copy_from_slice(token);
                    s.token_len = token.len();
                    self.sent = false; // again at once, with the token
                } else if status != w::STATUS_SUCCESS && status != STATUS_SAE_HASH_TO_ELEMENT {
                    self.refuse(Refusal::Status { frame: name, status });
                    return;
                } else {
                    match s.sae.process_commit(body) {
                        // Our confirm goes at once.
                        Ok(()) => { self.go(Step::SaeConfirm, now); return self.frames(b, r, dma, None, &[], None, now); }
                        Err(e) => { self.refuse(Refusal::Sae(e)); return; }
                    }
                }
            }
            (Step::SaeConfirm, Some(Heard::Auth { status, transaction: 2, algorithm: AUTH_SAE })) => {
                if status != w::STATUS_SUCCESS { self.refuse(Refusal::Status { frame: name, status }); return; }
                match self.sae.as_ref().unwrap().sae.check_confirm(body) {
                    Ok(()) => self.go(Step::Assoc, now),
                    Err(e) => self.refuse(Refusal::Sae(e)),
                }
                return;
            }
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
