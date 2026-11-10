// SPDX-License-Identifier: GPL-3.0-only
//! The station side of the 4-way and group-key handshakes (IEEE 802.11-2020 12.7.6/12.7.7)
//! for WPA2-Personal (AKM 2, PSK, SHA-1 keys) and WPA3-Personal (AKM 8, SAE, SHA-256 keys).
use crate::crypto::{aes_cmac, aes_unwrap, ct_eq, hmac_sha1, kdf_sha256, min_max, prf_sha1};

pub const ETHERTYPE: u16 = 0x888e;
const KEY_INFO_PAIRWISE: u16 = 1 << 3;
const KEY_INFO_INSTALL: u16 = 1 << 6;
const KEY_INFO_ACK: u16 = 1 << 7;
const KEY_INFO_MIC: u16 = 1 << 8;
const KEY_INFO_SECURE: u16 = 1 << 9;
const KEY_INFO_ENCRYPTED: u16 = 1 << 12;
/// EAPOL header (4) + descriptor (95 with a 16-byte MIC) before the key data.
const FIXED: usize = 4 + 95;
const MIC_AT: usize = 4 + 77;
pub const MAX_FRAME: usize = 512;
const MAX_KEY_DATA: usize = MAX_FRAME - FIXED;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Akm { Psk, Sae }
impl Akm {
    fn descriptor_version(self) -> u16 { match self { Akm::Psk => 2, Akm::Sae => 0 } }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { Short, NotKey, Replay, Mic, Unexpected, KeyData, RsnMismatch, NoGtk }

#[derive(Clone, Copy, Debug, Default)]
pub struct Ptk { pub kck: [u8; 16], pub kek: [u8; 16], pub tk: [u8; 16] }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroupKey { pub id: u8, pub key: [u8; 16], pub rsc: u64 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntegrityGroupKey { pub id: u16, pub key: [u8; 16], pub ipn: u64 }

/// Keys the driver installs into the firmware after a handshake step.
#[derive(Clone, Copy, Debug, Default)]
pub struct Install { pub pairwise: Option<[u8; 16]>, pub group: Option<GroupKey>, pub igtk: Option<IntegrityGroupKey> }

pub fn derive_ptk(akm: Akm, pmk: &[u8; 32], aa: &[u8; 6], spa: &[u8; 6], anonce: &[u8; 32], snonce: &[u8; 32]) -> Ptk {
    let (amin, amax) = min_max(aa, spa);
    let (nmin, nmax) = min_max(anonce, snonce);
    let mut data = [0u8; 76];
    data[..6].copy_from_slice(amin);
    data[6..12].copy_from_slice(amax);
    data[12..44].copy_from_slice(nmin);
    data[44..76].copy_from_slice(nmax);
    let mut out = [0u8; 48];
    match akm {
        Akm::Psk => prf_sha1(pmk, b"Pairwise key expansion", &data, &mut out),
        Akm::Sae => kdf_sha256(pmk, b"Pairwise key expansion", &data, &mut out),
    }
    let mut ptk = Ptk::default();
    ptk.kck.copy_from_slice(&out[..16]);
    ptk.kek.copy_from_slice(&out[16..32]);
    ptk.tk.copy_from_slice(&out[32..48]);
    ptk
}

fn mic(akm: Akm, kck: &[u8; 16], frame: &[u8]) -> [u8; 16] {
    match akm {
        Akm::Psk => { let h = hmac_sha1(kck, &[frame]); let mut m = [0u8; 16]; m.copy_from_slice(&h[..16]); m }
        Akm::Sae => aes_cmac(kck, &[frame]),
    }
}

/// A parsed EAPOL-Key frame (borrowing the receive buffer).
pub struct KeyFrame<'a> {
    pub info: u16,
    pub replay: u64,
    pub nonce: &'a [u8],
    pub rsc: u64,
    pub mic: &'a [u8],
    pub data: &'a [u8],
}

pub fn parse(frame: &[u8]) -> Result<KeyFrame<'_>, Error> {
    if frame.len() < FIXED { return Err(Error::Short); }
    if frame[1] != 3 || frame[4] != 2 { return Err(Error::NotKey); }
    let body = u16::from_be_bytes([frame[2], frame[3]]) as usize;
    if 4 + body > frame.len() || body < 95 { return Err(Error::Short); }
    let data_len = u16::from_be_bytes([frame[FIXED - 2], frame[FIXED - 1]]) as usize;
    if FIXED + data_len > 4 + body { return Err(Error::Short); }
    Ok(KeyFrame {
        info: u16::from_be_bytes([frame[5], frame[6]]),
        replay: u64::from_be_bytes(frame[9..17].try_into().unwrap()),
        nonce: &frame[17..49],
        rsc: u64::from_le_bytes(frame[65..73].try_into().unwrap()),
        mic: &frame[MIC_AT..MIC_AT + 16],
        data: &frame[FIXED..FIXED + data_len],
    })
}

/// Builds an EAPOL-Key frame from the station into `out`; returns its length.
#[allow(clippy::too_many_arguments)]
fn build(out: &mut [u8; MAX_FRAME], akm: Akm, info: u16, replay: u64, nonce: &[u8; 32], data: &[u8], kck: Option<&[u8; 16]>) -> usize {
    let len = FIXED + data.len();
    out[..len].fill(0);
    out[0] = 2; // 802.1X-2004
    out[1] = 3; // EAPOL-Key
    out[2..4].copy_from_slice(&((len - 4) as u16).to_be_bytes());
    out[4] = 2; // RSN key descriptor
    out[5..7].copy_from_slice(&(info | akm.descriptor_version()).to_be_bytes());
    out[9..17].copy_from_slice(&replay.to_be_bytes());
    out[17..49].copy_from_slice(nonce);
    out[FIXED - 2..FIXED].copy_from_slice(&(data.len() as u16).to_be_bytes());
    out[FIXED..len].copy_from_slice(data);
    if let Some(kck) = kck {
        let m = mic(akm, kck, &out[..len]);
        out[MIC_AT..MIC_AT + 16].copy_from_slice(&m);
    }
    len
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State { WaitMessage1, WaitMessage3, Done }

/// Station-side handshake state. `own_rsn` is the RSN element we sent in the
/// association request; `ap_rsn` is the one from the AP's beacon or probe
/// response, which message 3 must repeat exactly (downgrade protection).
pub struct Supplicant {
    akm: Akm,
    pmk: [u8; 32],
    aa: [u8; 6],
    spa: [u8; 6],
    snonce: [u8; 32],
    own_rsn: [u8; RSN_BYTES],
    own_rsn_len: u8,
    ap_rsn: [u8; RSN_BYTES],
    ap_rsn_len: u8,
    ptk: Option<Ptk>,
    replay: Option<u64>,
    /// Keys already handed to the driver. A retransmitted message 3 or group
    /// message (lost reply) is answered again but never reinstalls a key: that
    /// would reset packet numbers and replay counters (the KRACK attacks).
    installed_tk: Option<[u8; 16]>,
    installed_gtk: Option<[u8; 16]>,
    installed_igtk: Option<[u8; 16]>,
    pub state: State,
}

/// Longest RSN element kept (id, length and body).
pub const RSN_BYTES: usize = 64;

impl Supplicant {
    /// The elements are copied (cut at `RSN_BYTES`), so the state can be kept.
    pub fn new(akm: Akm, pmk: [u8; 32], aa: [u8; 6], spa: [u8; 6], snonce: [u8; 32], own_rsn: &[u8], ap_rsn: &[u8]) -> Self {
        let keep = |e: &[u8]| { let mut a = [0u8; RSN_BYTES]; let n = e.len().min(RSN_BYTES); a[..n].copy_from_slice(&e[..n]); (a, n as u8) };
        let ((own_rsn, own_rsn_len), (ap_rsn, ap_rsn_len)) = (keep(own_rsn), keep(ap_rsn));
        Self { akm, pmk, aa, spa, snonce, own_rsn, own_rsn_len, ap_rsn, ap_rsn_len, ptk: None, replay: None, installed_tk: None, installed_gtk: None, installed_igtk: None, state: State::WaitMessage1 }
    }
    pub fn ptk(&self) -> Option<&Ptk> { self.ptk.as_ref() }

    fn fresh_tk(&mut self, tk: [u8; 16]) -> Option<[u8; 16]> {
        if self.installed_tk.is_some_and(|k| ct_eq(&k, &tk)) { return None; }
        self.installed_tk = Some(tk);
        Some(tk)
    }
    fn fresh_gtk(&mut self, g: GroupKey) -> Option<GroupKey> {
        if self.installed_gtk.is_some_and(|k| ct_eq(&k, &g.key)) { return None; }
        self.installed_gtk = Some(g.key);
        Some(g)
    }
    fn fresh_igtk(&mut self, i: IntegrityGroupKey) -> Option<IntegrityGroupKey> {
        if self.installed_igtk.is_some_and(|k| ct_eq(&k, &i.key)) { return None; }
        self.installed_igtk = Some(i.key);
        Some(i)
    }

    fn check_replay(&mut self, replay: u64) -> Result<(), Error> {
        if let Some(last) = self.replay { if replay <= last { return Err(Error::Replay); } }
        Ok(())
    }

    fn verify_mic(&self, frame: &[u8], kck: &[u8; 16]) -> Result<(), Error> {
        let body = 4 + u16::from_be_bytes([frame[2], frame[3]]) as usize;
        let mut copy = [0u8; MAX_FRAME];
        if body > MAX_FRAME { return Err(Error::Short); }
        copy[..body].copy_from_slice(&frame[..body]);
        copy[MIC_AT..MIC_AT + 16].fill(0);
        let expect = mic(self.akm, kck, &copy[..body]);
        if ct_eq(&expect, &frame[MIC_AT..MIC_AT + 16]) { Ok(()) } else { Err(Error::Mic) }
    }

    fn decrypt_data(&self, kek: &[u8; 16], data: &[u8], out: &mut [u8; MAX_KEY_DATA]) -> Result<usize, Error> {
        if data.len() < 24 || data.len() % 8 != 0 || data.len() - 8 > MAX_KEY_DATA { return Err(Error::KeyData); }
        let n = data.len() - 8;
        aes_unwrap(kek, data, &mut out[..n]).map_err(|_| Error::Mic)?;
        Ok(n)
    }

    /// Handles one received EAPOL frame. Returns the reply length written to `reply`
    /// (0 if none) and the keys to install now.
    pub fn receive(&mut self, frame: &[u8], reply: &mut [u8; MAX_FRAME]) -> Result<(usize, Install), Error> {
        let key = parse(frame)?;
        let mut install = Install::default();
        if key.info & KEY_INFO_ACK == 0 { return Err(Error::Unexpected); }
        if key.info & KEY_INFO_PAIRWISE != 0 && key.info & KEY_INFO_MIC == 0 {
            // Message 1: may arrive again (retransmission or rekey); start over.
            self.check_replay(key.replay)?;
            let mut anonce = [0u8; 32];
            anonce.copy_from_slice(key.nonce);
            let ptk = derive_ptk(self.akm, &self.pmk, &self.aa, &self.spa, &anonce, &self.snonce);
            self.replay = Some(key.replay);
            let len = build(reply, self.akm, KEY_INFO_PAIRWISE | KEY_INFO_MIC, key.replay, &self.snonce, &self.own_rsn[..self.own_rsn_len as usize], Some(&ptk.kck));
            self.ptk = Some(ptk);
            self.state = State::WaitMessage3;
            return Ok((len, install));
        }
        let ptk = self.ptk.ok_or(Error::Unexpected)?;
        if key.info & KEY_INFO_MIC == 0 { return Err(Error::Unexpected); }
        self.check_replay(key.replay)?;
        self.verify_mic(frame, &ptk.kck)?;
        if key.info & KEY_INFO_ENCRYPTED == 0 { return Err(Error::KeyData); }
        let mut plain = [0u8; MAX_KEY_DATA];
        let n = self.decrypt_data(&ptk.kek, key.data, &mut plain)?;
        let kdes = &plain[..n];
        self.replay = Some(key.replay);
        if key.info & KEY_INFO_PAIRWISE != 0 {
            // Message 3.
            if self.state == State::WaitMessage1 { return Err(Error::Unexpected); }
            if key.info & KEY_INFO_INSTALL == 0 { return Err(Error::Unexpected); }
            let rsn = find_element(kdes, 48).ok_or(Error::RsnMismatch)?;
            if rsn != &self.ap_rsn[..self.ap_rsn_len as usize] { return Err(Error::RsnMismatch); }
            let gtk = find_gtk(kdes, key.rsc).ok_or(Error::NoGtk)?;
            install.pairwise = self.fresh_tk(ptk.tk);
            install.group = self.fresh_gtk(gtk);
            install.igtk = find_igtk(kdes).and_then(|i| self.fresh_igtk(i));
            let len = build(reply, self.akm, KEY_INFO_PAIRWISE | KEY_INFO_MIC | KEY_INFO_SECURE, key.replay, &[0; 32], &[], Some(&ptk.kck));
            self.state = State::Done;
            return Ok((len, install));
        }
        // Group key message 1 (rekey).
        if self.state != State::Done { return Err(Error::Unexpected); }
        install.group = self.fresh_gtk(find_gtk(kdes, key.rsc).ok_or(Error::NoGtk)?);
        install.igtk = find_igtk(kdes).and_then(|i| self.fresh_igtk(i));
        let len = build(reply, self.akm, KEY_INFO_MIC | KEY_INFO_SECURE, key.replay, &[0; 32], &[], Some(&ptk.kck));
        Ok((len, install))
    }
}

/// Iterates key data elements: (element id, body). Vendor KDEs have id 0xdd.
fn elements(data: &[u8]) -> impl Iterator<Item = (u8, &[u8])> {
    let mut rest = data;
    core::iter::from_fn(move || {
        if rest.len() < 2 || rest[0] == 0 && rest[1] == 0 { return None; }
        let id = rest[0];
        let len = rest[1] as usize;
        if 2 + len > rest.len() { return None; }
        let body = &rest[2..2 + len];
        rest = &rest[2 + len..];
        Some((id, body))
    })
}

/// Whole element (id, length and body) for the first element with this id.
fn find_element(data: &[u8], want: u8) -> Option<&[u8]> {
    let mut at = 0;
    while at + 2 <= data.len() {
        let len = data[at + 1] as usize;
        if at + 2 + len > data.len() { return None; }
        if data[at] == want { return Some(&data[at..at + 2 + len]); }
        at += 2 + len;
    }
    None
}

fn kde(data: &[u8], kind: u8) -> Option<&[u8]> {
    elements(data).find_map(|(id, body)| {
        (id == 0xdd && body.len() >= 4 && body[..3] == [0x00, 0x0f, 0xac] && body[3] == kind).then(|| &body[4..])
    })
}

fn find_gtk(data: &[u8], rsc: u64) -> Option<GroupKey> {
    let body = kde(data, 1)?;
    if body.len() != 2 + 16 { return None; }
    let mut key = [0u8; 16];
    key.copy_from_slice(&body[2..]);
    Some(GroupKey { id: body[0] & 3, key, rsc })
}

fn find_igtk(data: &[u8]) -> Option<IntegrityGroupKey> {
    let body = kde(data, 9)?;
    if body.len() != 2 + 6 + 16 { return None; }
    let mut ipn = [0u8; 8];
    ipn[..6].copy_from_slice(&body[2..8]);
    let mut key = [0u8; 16];
    key.copy_from_slice(&body[8..]);
    Some(IntegrityGroupKey { id: u16::from_le_bytes([body[0], body[1]]), key, ipn: u64::from_le_bytes(ipn) })
}

/// Authenticator-side helpers, used by the tests to play the access point and
/// later by SexOS when it shares its connection as a hotspot.
pub mod authenticator {
    use super::*;
    use crate::crypto::aes_wrap;

    pub fn message1(out: &mut [u8; MAX_FRAME], akm: Akm, replay: u64, anonce: &[u8; 32]) -> usize {
        build(out, akm, KEY_INFO_PAIRWISE | KEY_INFO_ACK, replay, anonce, &[], None)
    }

    /// Message 3 with the AP's RSN element, a GTK KDE and (optionally) an IGTK KDE.
    #[allow(clippy::too_many_arguments)]
    pub fn message3(out: &mut [u8; MAX_FRAME], akm: Akm, ptk: &Ptk, replay: u64, anonce: &[u8; 32], rsn: &[u8], gtk: &GroupKey, igtk: Option<&IntegrityGroupKey>) -> usize {
        let mut plain = [0u8; 256];
        let mut n = 0;
        plain[..rsn.len()].copy_from_slice(rsn);
        n += rsn.len();
        plain[n..n + 8].copy_from_slice(&[0xdd, 22, 0x00, 0x0f, 0xac, 1, gtk.id, 0]);
        n += 8;
        plain[n..n + 16].copy_from_slice(&gtk.key);
        n += 16;
        if let Some(i) = igtk {
            plain[n..n + 8].copy_from_slice(&[0xdd, 28, 0x00, 0x0f, 0xac, 9, i.id as u8, (i.id >> 8) as u8]);
            n += 8;
            plain[n..n + 6].copy_from_slice(&i.ipn.to_le_bytes()[..6]);
            n += 6;
            plain[n..n + 16].copy_from_slice(&i.key);
            n += 16;
        }
        // Pad to a multiple of 8 (and at least 16) with 0xdd 0x00...
        if n % 8 != 0 || n < 16 { plain[n] = 0xdd; n += 1; while n % 8 != 0 || n < 16 { plain[n] = 0; n += 1; } }
        let mut wrapped = [0u8; 264];
        aes_wrap(&ptk.kek, &plain[..n], &mut wrapped[..n + 8]).unwrap();
        let info = KEY_INFO_PAIRWISE | KEY_INFO_ACK | KEY_INFO_MIC | KEY_INFO_INSTALL | KEY_INFO_SECURE | KEY_INFO_ENCRYPTED;
        let len = build(out, akm, info, replay, anonce, &wrapped[..n + 8], Some(&ptk.kck));
        out[65..73].copy_from_slice(&gtk.rsc.to_le_bytes());
        // The RSC is covered by the MIC: recompute it.
        out[MIC_AT..MIC_AT + 16].fill(0);
        let m = mic(akm, &ptk.kck, &out[..len]);
        out[MIC_AT..MIC_AT + 16].copy_from_slice(&m);
        len
    }

    /// Group key message 1 (rekeying): the new GTK in a KDE, wrapped with the KEK.
    pub fn group_message1(out: &mut [u8; MAX_FRAME], akm: Akm, ptk: &Ptk, replay: u64, gtk: &GroupKey) -> usize {
        let mut plain = [0u8; 32];
        plain[..8].copy_from_slice(&[0xdd, 22, 0x00, 0x0f, 0xac, 1, gtk.id, 0]);
        plain[8..24].copy_from_slice(&gtk.key);
        plain[24] = 0xdd;
        let mut wrapped = [0u8; 40];
        aes_wrap(&ptk.kek, &plain, &mut wrapped).unwrap();
        let info = KEY_INFO_ACK | KEY_INFO_MIC | KEY_INFO_SECURE | KEY_INFO_ENCRYPTED;
        let len = build(out, akm, info, replay, &[0; 32], &wrapped, Some(&ptk.kck));
        out[65..73].copy_from_slice(&gtk.rsc.to_le_bytes());
        out[MIC_AT..MIC_AT + 16].fill(0);
        let m = mic(akm, &ptk.kck, &out[..len]);
        out[MIC_AT..MIC_AT + 16].copy_from_slice(&m);
        len
    }

    /// Checks a station's message 2 and returns its SNonce.
    pub fn check_message2(frame: &[u8], akm: Akm, pmk: &[u8; 32], aa: &[u8; 6], spa: &[u8; 6], anonce: &[u8; 32]) -> Result<(Ptk, [u8; 32]), Error> {
        let key = parse(frame)?;
        let mut snonce = [0u8; 32];
        snonce.copy_from_slice(key.nonce);
        let ptk = derive_ptk(akm, pmk, aa, spa, anonce, &snonce);
        let probe = Supplicant::new(akm, *pmk, *aa, *spa, snonce, &[], &[]);
        probe.verify_mic(frame, &ptk.kck)?;
        Ok((ptk, snonce))
    }

    pub fn check_message4(frame: &[u8], akm: Akm, ptk: &Ptk) -> Result<(), Error> {
        let key = parse(frame)?;
        if key.info & KEY_INFO_SECURE == 0 { return Err(Error::Unexpected); }
        let probe = Supplicant::new(akm, [0; 32], [0; 6], [0; 6], [0; 32], &[], &[]);
        probe.verify_mic(frame, &ptk.kck)
    }
}
