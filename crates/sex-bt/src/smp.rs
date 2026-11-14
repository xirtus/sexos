// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The LE Security Manager (Core 5.4, Vol 3 Part H) with LE Secure Connections: pairing
//! request/response, P-256 public keys, confirm and random values, numeric comparison
//! (or Just Works), DHKey checks and the long-term key. Both roles run the same code, so
//! the host tests pair two of these against each other.
//!
//! Not in W1: legacy (pre-4.2) pairing, passkey entry, OOB, and key distribution (we ask
//! for none; bonds keep the LTK only). A peer that needs them gets "pairing not supported".

use crate::crypto::{self, KeyPair};
use alloc::vec::Vec;

pub mod code {
    pub const PAIRING_REQUEST: u8 = 0x01;
    pub const PAIRING_RESPONSE: u8 = 0x02;
    pub const PAIRING_CONFIRM: u8 = 0x03;
    pub const PAIRING_RANDOM: u8 = 0x04;
    pub const PAIRING_FAILED: u8 = 0x05;
    pub const SECURITY_REQUEST: u8 = 0x0B;
    pub const PUBLIC_KEY: u8 = 0x0C;
    pub const DHKEY_CHECK: u8 = 0x0D;
}

pub mod reason {
    pub const AUTH_REQUIREMENTS: u8 = 0x03;
    pub const CONFIRM_VALUE_FAILED: u8 = 0x04;
    pub const PAIRING_NOT_SUPPORTED: u8 = 0x05;
    pub const ENCRYPTION_KEY_SIZE: u8 = 0x06;
    pub const UNSPECIFIED: u8 = 0x08;
    pub const INVALID_PARAMETERS: u8 = 0x0A;
    pub const DHKEY_CHECK_FAILED: u8 = 0x0B;
    pub const NUMERIC_COMPARISON_FAILED: u8 = 0x0C;
}

/// IO capabilities.
pub mod io {
    pub const DISPLAY_ONLY: u8 = 0;
    pub const DISPLAY_YES_NO: u8 = 1;
    pub const KEYBOARD_ONLY: u8 = 2;
    pub const NO_INPUT_NO_OUTPUT: u8 = 3;
    pub const KEYBOARD_DISPLAY: u8 = 4;
}

pub const AUTH_BONDING: u8 = 0x01;
pub const AUTH_MITM: u8 = 0x04;
pub const AUTH_SC: u8 = 0x08;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role { Initiator, Responder }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method { JustWorks, NumericComparison }

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    /// Send this SMP PDU on channel 6.
    Send(Vec<u8>),
    /// Show this six-digit value and ask the user whether it matches the other device;
    /// answer with `Smp::confirm`.
    Compare(u32),
    /// Initiator: pairing is done; start encryption with this LTK (rand 0, ediv 0).
    Encrypt([u8; 16]),
    /// Responder: pairing is done; answer the LE LTK request with this key.
    Ltk([u8; 16]),
    /// Pairing ended with this reason (sent or received).
    Failed(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State { Idle, Response, PublicKey, Confirm, Random, User, Check, Done }

pub struct Smp {
    role: Role,
    io: u8,
    auth: u8,
    /// Our address and the peer's, each six bytes then the address type.
    local: [u8; 7],
    remote: [u8; 7],
    keys: KeyPair,
    nonce: [u8; 16],
    preq: [u8; 7],
    pres: [u8; 7],
    peer_pk: [u8; 64],
    dhkey: [u8; 32],
    peer_confirm: [u8; 16],
    peer_nonce: [u8; 16],
    mackey: [u8; 16],
    ltk: [u8; 16],
    method: Method,
    state: State,
    user_ok: bool,
    peer_checked: bool,
}

fn arr<const N: usize>(b: &[u8]) -> [u8; N] { let mut a = [0; N]; a.copy_from_slice(&b[..N]); a }
fn x(pk: &[u8; 64]) -> [u8; 32] { arr(&pk[..32]) }

impl Smp {
    /// `secret` and `nonce` are fresh random bytes from the caller's entropy source.
    /// None if `secret` is not a usable private key (draw again).
    pub fn new(role: Role, io: u8, auth: u8, local: [u8; 7], remote: [u8; 7], secret: &[u8; 32], nonce: [u8; 16]) -> Option<Smp> {
        Some(Smp { role, io, auth: auth | AUTH_SC, local, remote, keys: KeyPair::from_secret(secret)?, nonce,
            preq: [0; 7], pres: [0; 7], peer_pk: [0; 64], dhkey: [0; 32], peer_confirm: [0; 16], peer_nonce: [0; 16],
            mackey: [0; 16], ltk: [0; 16], method: Method::JustWorks, state: State::Idle, user_ok: false, peer_checked: false })
    }

    pub fn method(&self) -> Method { self.method }
    pub fn done(&self) -> bool { self.state == State::Done }

    fn features(&self, code: u8) -> [u8; 7] { [code, self.io, 0, self.auth, 16, 0, 0] }

    /// Initiator: the Pairing Request that starts pairing.
    pub fn start(&mut self) -> Vec<Action> {
        if self.role != Role::Initiator || self.state != State::Idle { return Vec::new(); }
        self.preq = self.features(code::PAIRING_REQUEST);
        self.state = State::Response;
        alloc::vec![Action::Send(self.preq.to_vec())]
    }

    fn fail(&mut self, r: u8) -> Vec<Action> {
        self.state = State::Done;
        alloc::vec![Action::Send(alloc::vec![code::PAIRING_FAILED, r]), Action::Failed(r)]
    }

    /// Choose the method from both sides' features; None: we cannot pair this way.
    fn choose(&self) -> Result<Method, u8> {
        let (a, b) = (&self.preq, &self.pres);
        if a[3] & AUTH_SC == 0 || b[3] & AUTH_SC == 0 { return Err(reason::AUTH_REQUIREMENTS); }
        if a[4].min(b[4]) < 7 { return Err(reason::ENCRYPTION_KEY_SIZE); }
        let mitm = (a[3] | b[3]) & AUTH_MITM != 0;
        let yes_no = |c: u8| c == io::DISPLAY_YES_NO || c == io::KEYBOARD_DISPLAY;
        let input = |c: u8| c == io::KEYBOARD_ONLY || c == io::KEYBOARD_DISPLAY;
        let display = |c: u8| c != io::NO_INPUT_NO_OUTPUT && c != io::KEYBOARD_ONLY;
        if a[1] > 4 || b[1] > 4 { return Err(reason::INVALID_PARAMETERS); }
        if !mitm { return Ok(Method::JustWorks); }
        if yes_no(a[1]) && yes_no(b[1]) { return Ok(Method::NumericComparison); }
        // Passkey entry (one side types what the other shows) arrives with W2.
        if (input(a[1]) && display(b[1])) || (input(b[1]) && display(a[1])) { return Err(reason::PAIRING_NOT_SUPPORTED); }
        Ok(Method::JustWorks)
    }

    /// (A, B, Na, Nb, PKax, PKbx): initiator-side values whichever role we are.
    fn sides(&self) -> ([u8; 7], [u8; 7], [u8; 16], [u8; 16], [u8; 32], [u8; 32]) {
        match self.role {
            Role::Initiator => (self.local, self.remote, self.nonce, self.peer_nonce, x(&self.keys.public), x(&self.peer_pk)),
            Role::Responder => (self.remote, self.local, self.peer_nonce, self.nonce, x(&self.peer_pk), x(&self.keys.public)),
        }
    }

    fn derive(&mut self) {
        let (a, b, na, nb, _, _) = self.sides();
        (self.mackey, self.ltk) = crypto::f5(&self.dhkey, &na, &nb, &a, &b);
    }

    fn own_check(&self) -> [u8; 16] {
        let io: [u8; 3] = arr(&if self.role == Role::Initiator { self.preq } else { self.pres }[1..4]);
        crypto::f6(&self.mackey, &self.nonce, &self.peer_nonce, &[0; 16], &io, &self.local, &self.remote)
    }

    fn peer_check(&self) -> [u8; 16] {
        let io: [u8; 3] = arr(&if self.role == Role::Initiator { self.pres } else { self.preq }[1..4]);
        crypto::f6(&self.mackey, &self.peer_nonce, &self.nonce, &[0; 16], &io, &self.remote, &self.local)
    }

    fn compare_value(&self) -> u32 {
        let (_, _, na, nb, pkax, pkbx) = self.sides();
        crypto::g2(&pkax, &pkbx, &na, &nb)
    }

    fn public_key_pdu(&self) -> Vec<u8> {
        let mut p = alloc::vec![code::PUBLIC_KEY];
        p.extend_from_slice(&self.keys.public);
        p
    }

    fn check_pdu(&self) -> Vec<u8> {
        let mut p = alloc::vec![code::DHKEY_CHECK];
        p.extend_from_slice(&self.own_check());
        p
    }

    /// Responder: both checks are settled (peer's Ea verified, user said yes).
    fn responder_finish(&mut self) -> Vec<Action> {
        self.state = State::Done;
        alloc::vec![Action::Send(self.check_pdu()), Action::Ltk(self.ltk)]
    }

    /// The user's answer to `Action::Compare`.
    pub fn confirm(&mut self, matches: bool) -> Vec<Action> {
        if self.state == State::Done || self.method != Method::NumericComparison { return Vec::new(); }
        if !matches { return self.fail(reason::NUMERIC_COMPARISON_FAILED); }
        self.user_ok = true;
        match self.role {
            Role::Initiator if self.state == State::User => {
                self.state = State::Check;
                alloc::vec![Action::Send(self.check_pdu())]
            }
            Role::Responder if self.peer_checked => self.responder_finish(),
            _ => Vec::new(),
        }
    }

    /// One SMP PDU from the peer.
    pub fn on_pdu(&mut self, pdu: &[u8]) -> Vec<Action> {
        let Some(&c) = pdu.first() else { return Vec::new() };
        if self.state == State::Done { return Vec::new(); }
        let body = &pdu[1..];
        if c == code::PAIRING_FAILED {
            self.state = State::Done;
            return alloc::vec![Action::Failed(body.first().copied().unwrap_or(reason::UNSPECIFIED))];
        }
        match (self.role, self.state, c) {
            (Role::Responder, State::Idle, code::PAIRING_REQUEST) if body.len() == 6 => {
                self.preq = arr(pdu);
                self.pres = self.features(code::PAIRING_RESPONSE);
                self.pres[4] = self.pres[4].min(self.preq[4]);
                match self.choose() {
                    Ok(m) => self.method = m,
                    Err(r) => return self.fail(r),
                }
                self.state = State::PublicKey;
                alloc::vec![Action::Send(self.pres.to_vec())]
            }
            (Role::Initiator, State::Response, code::PAIRING_RESPONSE) if body.len() == 6 => {
                self.pres = arr(pdu);
                match self.choose() {
                    Ok(m) => self.method = m,
                    Err(r) => return self.fail(r),
                }
                self.state = State::PublicKey;
                alloc::vec![Action::Send(self.public_key_pdu())]
            }
            (_, State::PublicKey, code::PUBLIC_KEY) if body.len() == 64 => {
                self.peer_pk = arr(body);
                // A peer echoing our own key back is an attack (CVE-2020-26558 class).
                if self.peer_pk == self.keys.public { return self.fail(reason::INVALID_PARAMETERS); }
                let Some(dh) = self.keys.dhkey(&self.peer_pk) else { return self.fail(reason::DHKEY_CHECK_FAILED) };
                self.dhkey = dh;
                match self.role {
                    Role::Initiator => { self.state = State::Confirm; Vec::new() }
                    Role::Responder => {
                        let cb = crypto::f4(&x(&self.keys.public), &x(&self.peer_pk), &self.nonce, 0);
                        let mut p = alloc::vec![code::PAIRING_CONFIRM];
                        p.extend_from_slice(&cb);
                        self.state = State::Random;
                        alloc::vec![Action::Send(self.public_key_pdu()), Action::Send(p)]
                    }
                }
            }
            (Role::Initiator, State::Confirm, code::PAIRING_CONFIRM) if body.len() == 16 => {
                self.peer_confirm = arr(body);
                self.state = State::Random;
                let mut p = alloc::vec![code::PAIRING_RANDOM];
                p.extend_from_slice(&self.nonce);
                alloc::vec![Action::Send(p)]
            }
            (Role::Initiator, State::Random, code::PAIRING_RANDOM) if body.len() == 16 => {
                self.peer_nonce = arr(body);
                if crypto::f4(&x(&self.peer_pk), &x(&self.keys.public), &self.peer_nonce, 0) != self.peer_confirm {
                    return self.fail(reason::CONFIRM_VALUE_FAILED);
                }
                self.derive();
                if self.method == Method::NumericComparison {
                    self.state = State::User;
                    alloc::vec![Action::Compare(self.compare_value())]
                } else {
                    self.state = State::Check;
                    alloc::vec![Action::Send(self.check_pdu())]
                }
            }
            (Role::Responder, State::Random, code::PAIRING_RANDOM) if body.len() == 16 => {
                self.peer_nonce = arr(body);
                self.derive();
                self.state = State::Check;
                let mut p = alloc::vec![code::PAIRING_RANDOM];
                p.extend_from_slice(&self.nonce);
                let mut a = alloc::vec![Action::Send(p)];
                if self.method == Method::NumericComparison { a.push(Action::Compare(self.compare_value())); }
                a
            }
            (Role::Responder, State::Check, code::DHKEY_CHECK) if body.len() == 16 => {
                if body != self.peer_check() { return self.fail(reason::DHKEY_CHECK_FAILED); }
                self.peer_checked = true;
                if self.method == Method::JustWorks || self.user_ok { self.responder_finish() } else { Vec::new() }
            }
            (Role::Initiator, State::Check, code::DHKEY_CHECK) if body.len() == 16 => {
                if body != self.peer_check() { return self.fail(reason::DHKEY_CHECK_FAILED); }
                self.state = State::Done;
                alloc::vec![Action::Encrypt(self.ltk)]
            }
            // A Security Request while pairing, or a PDU out of turn.
            (_, _, code::SECURITY_REQUEST) => Vec::new(),
            _ => self.fail(reason::UNSPECIFIED),
        }
    }
}
