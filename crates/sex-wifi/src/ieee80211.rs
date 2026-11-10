// SPDX-License-Identifier: GPL-3.0-only
//! 802.11 frames the station builds and reads: scan results, authentication
//! (open and SAE), association, and data frames to and from Ethernet II, so the
//! existing Ethernet/IP stack in sexnet runs unchanged on top of Wi-Fi.

pub const BROADCAST: [u8; 6] = [0xff; 6];
pub const FC_MGMT_ASSOC_REQ: u16 = 0x0000;
pub const FC_MGMT_ASSOC_RESP: u16 = 0x0010;
pub const FC_MGMT_PROBE_REQ: u16 = 0x0040;
pub const FC_MGMT_PROBE_RESP: u16 = 0x0050;
pub const FC_MGMT_BEACON: u16 = 0x0080;
pub const FC_MGMT_DISASSOC: u16 = 0x00a0;
pub const FC_MGMT_AUTH: u16 = 0x00b0;
pub const FC_MGMT_DEAUTH: u16 = 0x00c0;
pub const FC_DATA: u16 = 0x0008;
pub const FC_QOS_DATA: u16 = 0x0088;
pub const FC_TO_DS: u16 = 0x0100;
pub const FC_FROM_DS: u16 = 0x0200;
pub const FC_PROTECTED: u16 = 0x4000;
pub const MGMT_HEADER: usize = 24;
const LLC_SNAP: [u8; 6] = [0xaa, 0xaa, 0x03, 0x00, 0x00, 0x00];

pub const AUTH_OPEN: u16 = 0;
pub const AUTH_SAE: u16 = 3;
pub const STATUS_SUCCESS: u16 = 0;
pub const STATUS_ANTI_CLOGGING: u16 = 76;
pub const STATUS_SAE_HASH_TO_ELEMENT: u16 = 126;

const SUITE_CCMP: [u8; 4] = [0x00, 0x0f, 0xac, 4];
const SUITE_TKIP: [u8; 4] = [0x00, 0x0f, 0xac, 2];
const SUITE_BIP_CMAC: [u8; 4] = [0x00, 0x0f, 0xac, 6];
const AKM_PSK: [u8; 4] = [0x00, 0x0f, 0xac, 2];
const AKM_SAE: [u8; 4] = [0x00, 0x0f, 0xac, 8];
const RSN_MFPR: u16 = 1 << 6;
const RSN_MFPC: u16 = 1 << 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Security { Open, Wpa2Personal, Wpa3Personal, Wpa3Transition, Unsupported }

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rsn { pub group_ccmp: bool, pub pairwise_ccmp: bool, pub psk: bool, pub sae: bool, pub mfpc: bool, pub mfpr: bool }

pub fn parse_rsn(body: &[u8]) -> Option<Rsn> {
    if body.len() < 2 || u16::from_le_bytes([body[0], body[1]]) != 1 { return None; }
    let mut r = Rsn::default();
    let mut at = 2;
    if body.len() < at + 4 { r.group_ccmp = true; r.pairwise_ccmp = true; r.psk = false; return Some(r); }
    let group: [u8; 4] = body[at..at + 4].try_into().ok()?;
    r.group_ccmp = group == SUITE_CCMP;
    if group != SUITE_CCMP && group != SUITE_TKIP { return Some(Rsn { group_ccmp: false, ..r }); }
    at += 4;
    let count = u16::from_le_bytes(body.get(at..at + 2)?.try_into().ok()?) as usize;
    at += 2;
    for _ in 0..count {
        if body.get(at..at + 4)? == SUITE_CCMP { r.pairwise_ccmp = true; }
        at += 4;
    }
    let count = u16::from_le_bytes(body.get(at..at + 2)?.try_into().ok()?) as usize;
    at += 2;
    for _ in 0..count {
        let suite = body.get(at..at + 4)?;
        if suite == AKM_PSK { r.psk = true; }
        if suite == AKM_SAE { r.sae = true; }
        at += 4;
    }
    if let Some(caps) = body.get(at..at + 2) {
        let caps = u16::from_le_bytes([caps[0], caps[1]]);
        r.mfpr = caps & RSN_MFPR != 0;
        r.mfpc = caps & RSN_MFPC != 0;
    }
    Some(r)
}

impl Rsn {
    pub fn security(&self) -> Security {
        if !self.pairwise_ccmp { return Security::Unsupported; }
        match (self.psk, self.sae) {
            (true, true) => Security::Wpa3Transition,
            (false, true) => Security::Wpa3Personal,
            (true, false) => Security::Wpa2Personal,
            _ => Security::Unsupported,
        }
    }
}

/// The RSN element we put in the association request and EAPOL message 2.
/// WPA3 always asks for protected management frames (MFPR); WPA2 offers them.
pub fn own_rsn(sae: bool, out: &mut [u8; 32]) -> usize {
    let mut n = 0;
    let mut put = |bytes: &[u8]| { out[n..n + bytes.len()].copy_from_slice(bytes); n += bytes.len(); };
    put(&[48, 0]);
    put(&1u16.to_le_bytes());
    put(&SUITE_CCMP);
    put(&1u16.to_le_bytes());
    put(&SUITE_CCMP);
    put(&1u16.to_le_bytes());
    put(if sae { &AKM_SAE } else { &AKM_PSK });
    let caps = if sae { RSN_MFPC | RSN_MFPR } else { RSN_MFPC };
    put(&caps.to_le_bytes());
    if sae {
        put(&0u16.to_le_bytes()); // no PMKIDs
        put(&SUITE_BIP_CMAC);
    }
    out[1] = (n - 2) as u8;
    n
}

/// Iterates information elements: (id, body).
pub fn elements(data: &[u8]) -> impl Iterator<Item = (u8, &[u8])> {
    let mut rest = data;
    core::iter::from_fn(move || {
        if rest.len() < 2 { return None; }
        let len = rest[1] as usize;
        if 2 + len > rest.len() { return None; }
        let item = (rest[0], &rest[2..2 + len]);
        rest = &rest[2 + len..];
        Some(item)
    })
}

/// One access point seen in a scan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bss {
    pub bssid: [u8; 6],
    pub ssid: [u8; 32],
    pub ssid_len: u8,
    pub channel: u8,
    pub security: Security,
    pub rsn: Rsn,
    /// The AP's RSN element (id, length, body), which EAPOL message 3 must repeat.
    pub rsn_element: [u8; 64],
    pub rsn_element_len: u8,
    pub beacon_interval: u16,
    pub capability: u16,
    pub h2e: bool,
    /// Basic (required) rates the AP named, as rate-index bits: CCK 1, 2, 5.5, 11
    /// Mb/s in bits 0..4, OFDM 6..54 Mb/s in bits 0..8 (`join::ack_rates` reads them).
    pub basic_cck: u8,
    pub basic_ofdm: u8,
    /// DTIM period from the TIM element (0 when the frame had none).
    pub dtim_period: u8,
}
/// Rates in 500 kb/s units, in iwx(4) rate-index order: CCK then OFDM.
pub const CCK_RATES: [u8; 4] = [2, 4, 11, 22];
pub const OFDM_RATES: [u8; 8] = [12, 18, 24, 36, 48, 72, 96, 108];
impl Bss {
    pub fn ssid(&self) -> &[u8] { &self.ssid[..self.ssid_len as usize] }
    pub fn rsn_element(&self) -> &[u8] { &self.rsn_element[..self.rsn_element_len as usize] }
}

/// Reads a beacon or probe response (full frame including the 24-byte header).
pub fn parse_bss(frame: &[u8]) -> Option<Bss> {
    if frame.len() < MGMT_HEADER + 12 { return None; }
    let fc = u16::from_le_bytes([frame[0], frame[1]]);
    if fc & 0x00fc != FC_MGMT_BEACON && fc & 0x00fc != FC_MGMT_PROBE_RESP { return None; }
    let mut bss = Bss { bssid: frame[16..22].try_into().unwrap(), ssid: [0; 32], ssid_len: 0, channel: 0,
        security: Security::Open, rsn: Rsn::default(), rsn_element: [0; 64], rsn_element_len: 0,
        beacon_interval: u16::from_le_bytes([frame[32], frame[33]]),
        capability: u16::from_le_bytes([frame[34], frame[35]]), h2e: false,
        basic_cck: 0, basic_ofdm: 0, dtim_period: 0 };
    let privacy = bss.capability & 0x0010 != 0;
    let mut has_rsn = false;
    for (id, body) in elements(&frame[MGMT_HEADER + 12..]) {
        match id {
            0 if body.len() <= 32 => { bss.ssid[..body.len()].copy_from_slice(body); bss.ssid_len = body.len() as u8; }
            3 if body.len() == 1 => bss.channel = body[0],
            1 | 50 => for &r in body.iter().filter(|r| *r & 0x80 != 0) {
                let r = r & 0x7f;
                if let Some(i) = CCK_RATES.iter().position(|&c| c == r) { bss.basic_cck |= 1 << i; }
                if let Some(i) = OFDM_RATES.iter().position(|&c| c == r) { bss.basic_ofdm |= 1 << i; }
            },
            5 if body.len() >= 2 => bss.dtim_period = body[1],
            61 if !body.is_empty() && bss.channel == 0 => bss.channel = body[0], // HT operation primary channel
            48 if body.len() + 2 <= 64 => {
                has_rsn = true;
                bss.rsn_element[0] = 48;
                bss.rsn_element[1] = body.len() as u8;
                bss.rsn_element[2..2 + body.len()].copy_from_slice(body);
                bss.rsn_element_len = body.len() as u8 + 2;
                bss.rsn = parse_rsn(body).unwrap_or_default();
            }
            244 => bss.h2e = body.first().is_some_and(|b| b & (1 << 5) != 0), // RSNXE: SAE hash-to-element
            _ => {}
        }
    }
    bss.security = if has_rsn { bss.rsn.security() } else if privacy { Security::Unsupported } else { Security::Open };
    Some(bss)
}

fn mgmt_header(out: &mut [u8], fc: u16, da: &[u8; 6], sa: &[u8; 6], bssid: &[u8; 6], seq: u16) {
    out[..2].copy_from_slice(&fc.to_le_bytes());
    out[2..4].fill(0);
    out[4..10].copy_from_slice(da);
    out[10..16].copy_from_slice(sa);
    out[16..22].copy_from_slice(bssid);
    out[22..24].copy_from_slice(&(seq << 4).to_le_bytes());
}

/// Authentication frame. `body` is empty for open, or an SAE commit/confirm.
pub fn auth(out: &mut [u8], bssid: &[u8; 6], sa: &[u8; 6], seq: u16, algorithm: u16, transaction: u16, status: u16, body: &[u8]) -> usize {
    mgmt_header(out, FC_MGMT_AUTH, bssid, sa, bssid, seq);
    out[24..26].copy_from_slice(&algorithm.to_le_bytes());
    out[26..28].copy_from_slice(&transaction.to_le_bytes());
    out[28..30].copy_from_slice(&status.to_le_bytes());
    out[30..30 + body.len()].copy_from_slice(body);
    30 + body.len()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthReply<'a> { pub algorithm: u16, pub transaction: u16, pub status: u16, pub body: &'a [u8] }
pub fn parse_auth(frame: &[u8]) -> Option<AuthReply<'_>> {
    if frame.len() < 30 || u16::from_le_bytes([frame[0], frame[1]]) & 0x00fc != FC_MGMT_AUTH { return None; }
    Some(AuthReply {
        algorithm: u16::from_le_bytes([frame[24], frame[25]]),
        transaction: u16::from_le_bytes([frame[26], frame[27]]),
        status: u16::from_le_bytes([frame[28], frame[29]]),
        body: &frame[30..],
    })
}

/// Association request with SSID, rates and our RSN element.
pub fn assoc_request(out: &mut [u8], bss: &Bss, sa: &[u8; 6], seq: u16, rsn: &[u8]) -> usize {
    mgmt_header(out, FC_MGMT_ASSOC_REQ, &bss.bssid, sa, &bss.bssid, seq);
    let capability = 0x0001 | if bss.security != Security::Open { 0x0010 } else { 0 } | 0x0400; // ESS, privacy, short slot
    out[24..26].copy_from_slice(&(capability as u16).to_le_bytes());
    out[26..28].copy_from_slice(&10u16.to_le_bytes()); // listen interval
    let mut n = 28;
    out[n] = 0; out[n + 1] = bss.ssid_len; n += 2;
    out[n..n + bss.ssid_len as usize].copy_from_slice(bss.ssid()); n += bss.ssid_len as usize;
    // 2.4 GHz rates when the channel says so; OFDM rates otherwise (basic rates marked).
    let rates: &[u8] = if bss.channel != 0 && bss.channel <= 14 {
        &[1, 8, 0x82, 0x84, 0x8b, 0x96, 0x0c, 0x12, 0x18, 0x24, 50, 4, 0x30, 0x48, 0x60, 0x6c]
    } else {
        &[1, 8, 0x8c, 0x12, 0x98, 0x24, 0xb0, 0x48, 0x60, 0x6c]
    };
    out[n..n + rates.len()].copy_from_slice(rates); n += rates.len();
    out[n..n + rsn.len()].copy_from_slice(rsn); n += rsn.len();
    if bss.h2e {
        out[n..n + 3].copy_from_slice(&[244, 1, 1 << 5]); n += 3; // RSNXE: we do hash-to-element too
    }
    n
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AssocReply { pub status: u16, pub aid: u16 }
pub fn parse_assoc_response(frame: &[u8]) -> Option<AssocReply> {
    if frame.len() < 30 || u16::from_le_bytes([frame[0], frame[1]]) & 0x00fc != FC_MGMT_ASSOC_RESP { return None; }
    Some(AssocReply { status: u16::from_le_bytes([frame[26], frame[27]]), aid: u16::from_le_bytes([frame[28], frame[29]]) & 0x3fff })
}

/// Deauthentication or disassociation reason, if this is one.
pub fn parse_disconnect(frame: &[u8]) -> Option<u16> {
    if frame.len() < 26 { return None; }
    let kind = u16::from_le_bytes([frame[0], frame[1]]) & 0x00fc;
    (kind == FC_MGMT_DEAUTH || kind == FC_MGMT_DISASSOC).then(|| u16::from_le_bytes([frame[24], frame[25]]))
}

/// Ethernet II frame from the IP stack to an 802.11 data frame for the AP (to-DS).
/// Returns the 802.11 header length and total length written to `out`.
pub fn from_ethernet(ethernet: &[u8], bssid: &[u8; 6], seq: u16, out: &mut [u8]) -> Option<(usize, usize)> {
    if ethernet.len() < 14 { return None; }
    let payload = &ethernet[12..];
    let total = 24 + LLC_SNAP.len() + payload.len();
    if out.len() < total { return None; }
    out[..2].copy_from_slice(&(FC_DATA | FC_TO_DS).to_le_bytes());
    out[2..4].fill(0);
    out[4..10].copy_from_slice(bssid);
    out[10..16].copy_from_slice(&ethernet[6..12]);
    out[16..22].copy_from_slice(&ethernet[..6]);
    out[22..24].copy_from_slice(&(seq << 4).to_le_bytes());
    out[24..30].copy_from_slice(&LLC_SNAP);
    out[30..total].copy_from_slice(payload);
    Some((24, total))
}

/// Header length of a data frame (with QoS control and a fourth address when present).
pub fn data_header_len(frame: &[u8]) -> Option<usize> {
    if frame.len() < 24 { return None; }
    let fc = u16::from_le_bytes([frame[0], frame[1]]);
    if (fc >> 2) & 3 != 2 { return None; }
    let mut n = 24;
    if fc & 0x0300 == 0x0300 { n += 6; }
    if fc & 0x0080 != 0 { n += 2; }
    if fc & 0x8000 != 0 && fc & 0x0080 != 0 { n += 4; } // HT control
    (frame.len() >= n).then_some(n)
}

/// 802.11 data frame body (after decryption) from the AP (from-DS) to Ethernet II.
/// `header` is the 802.11 header; `body` the LLC/SNAP payload. Returns the length written.
pub fn to_ethernet(header: &[u8], body: &[u8], out: &mut [u8]) -> Option<usize> {
    if header.len() < 24 || body.len() < 8 || body[..6] != LLC_SNAP { return None; }
    let fc = u16::from_le_bytes([header[0], header[1]]);
    let (da, sa) = match fc & 0x0300 {
        FC_FROM_DS => (&header[4..10], &header[16..22]),
        0 => (&header[4..10], &header[10..16]),
        _ => return None,
    };
    let rest = &body[6..];
    let total = 12 + rest.len();
    if out.len() < total { return None; }
    out[..6].copy_from_slice(da);
    out[6..12].copy_from_slice(sa);
    out[12..total].copy_from_slice(rest);
    Some(total)
}

/// Channel number to centre frequency in MHz (2.4, 5 and 6 GHz bands).
pub fn channel_mhz(channel: u8, six_ghz: bool) -> u16 {
    let c = channel as u16;
    if six_ghz { return if c == 2 { 5935 } else { 5950 + 5 * c }; }
    match c { 14 => 2484, 1..=13 => 2407 + 5 * c, _ => 5000 + 5 * c }
}
