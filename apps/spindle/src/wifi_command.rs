// SPDX-License-Identifier: GPL-3.0-only
//! `wifi`: what sexnet's Intel Wi-Fi chip is doing and the networks its last scan
//! heard (`sex_pdx::wifi_status`). `wifi scan` asks for a fresh scan.
//!
//! sexnet answers each request with one reply word, which `receive` turns into
//! lines; a network takes a few words (its facts, its BSSID, its SSID in 7-byte
//! parts), asked one after another.
use sex_pdx::wifi_status as ws;

/// sexnet's PD.
pub const NET_PD: u32 = 13;
/// The most networks one listing shows (sexnet keeps 32).
const MAX_LISTED: u64 = 64;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Waiting {
    Nothing,
    State { scan: bool },
    Network { index: u64, part: u64 },
}

pub struct Wifi {
    waiting: Waiting,
    info: ws::Info,
    bssid: [u8; 7],
    ssid: [u8; 35],
}
pub static mut WIFI: Wifi = Wifi {
    waiting: Waiting::Nothing,
    info: ws::Info { channel: 0, rssi: 0, security: 0, ssid_len: 0, beacons: 0 },
    bssid: [0; 7],
    ssid: [0; 35],
};

const USAGE: &[u8] = b"Use: wifi [scan]";

fn ask(action: u64, a: u64, b: u64) -> bool {
    let (status, _) = sex_pdx::pdx_call(sex_pdx::SLOT_NET, ws::OP, action, a, b);
    status == 0
}

fn put(out: &mut [u8], at: &mut usize, text: &[u8]) {
    for &b in text { if *at < out.len() { out[*at] = b; *at += 1; } }
}

fn number(out: &mut [u8], at: &mut usize, n: i64) {
    if n < 0 { put(out, at, b"-"); }
    let mut v = n.unsigned_abs();
    let mut digits = [0u8; 20];
    let mut len = 0;
    loop { digits[len] = b'0' + (v % 10) as u8; len += 1; v /= 10; if v == 0 { break; } }
    for i in (0..len).rev() { put(out, at, &[digits[i]]); }
}

fn hex(out: &mut [u8], at: &mut usize, b: u8) {
    const H: &[u8; 16] = b"0123456789abcdef";
    put(out, at, &[H[(b >> 4) as usize], H[(b & 15) as usize]]);
}

/// The state line for a state word.
fn state_line(s: ws::State, scan: bool, out: &mut [u8]) -> usize {
    let mut at = 0;
    match s.phase {
        ws::NONE => put(out, &mut at, b"Wi-Fi: no Intel Wi-Fi chip here, or its firmware is missing."),
        ws::STARTING => put(out, &mut at, b"Wi-Fi: starting the chip's firmware; try again in a moment."),
        ws::SCANNING => {
            put(out, &mut at, if scan { b"Wi-Fi: scanning " } else { b"Wi-Fi: still scanning " });
            number(out, &mut at, s.channels as i64);
            put(out, &mut at, b" channels (");
            number(out, &mut at, s.networks as i64);
            put(out, &mut at, b" networks so far). Type wifi to see them.");
        }
        ws::READY => {
            put(out, &mut at, b"Wi-Fi: ");
            number(out, &mut at, s.networks as i64);
            put(out, &mut at, if s.networks == 1 { b" network heard" } else { b" networks heard" });
            put(out, &mut at, b" on ");
            number(out, &mut at, s.channels as i64);
            put(out, &mut at, b" channels (scan ");
            number(out, &mut at, s.scans as i64);
            put(out, &mut at, b").");
            if scan { put(out, &mut at, b" A scan is not possible right now."); }
        }
        _ => put(out, &mut at, b"Wi-Fi: the chip stopped; the boot log's [wifi.failed] line says where."),
    }
    at
}

/// One network's line.
fn network_line(info: ws::Info, bssid: &[u8], ssid: &[u8], out: &mut [u8]) -> usize {
    let mut at = 0;
    put(out, &mut at, b"  ");
    if ssid.is_empty() || ssid.iter().all(|&b| b == 0) {
        put(out, &mut at, b"(hidden)");
    } else {
        for &b in ssid { put(out, &mut at, &[if (0x20..0x7f).contains(&b) { b } else { b'?' }]); }
    }
    put(out, &mut at, b"  ");
    number(out, &mut at, info.rssi as i64);
    put(out, &mut at, b" dBm  channel ");
    number(out, &mut at, info.channel as i64);
    put(out, &mut at, b"  ");
    put(out, &mut at, ws::security_name(info.security).as_bytes());
    put(out, &mut at, b"  ");
    for (i, &b) in bssid.iter().take(6).enumerate() {
        if i > 0 { put(out, &mut at, b":"); }
        hex(out, &mut at, b);
    }
    at
}

impl Wifi {
    /// Starts a `wifi` command; the answer arrives through `receive`.
    pub fn start(&mut self, args: &[u8], sb: &mut crate::Scrollback) {
        let mut words = args.split(|&b| b == b' ').filter(|w| !w.is_empty());
        let (action, scan) = match (words.next(), words.next()) {
            (None, _) => (ws::STATE, false),
            (Some(b"scan"), None) => (ws::SCAN, true),
            _ => { sb.push(USAGE); return; }
        };
        if ask(action, 0, 0) { self.waiting = Waiting::State { scan }; } else { sb.push(b"Wi-Fi: the network service is unreachable."); }
    }

    /// A reply from sexnet. False when it was not for a `wifi` command.
    pub fn receive(&mut self, caller: u32, value: u64, sb: &mut crate::Scrollback) -> bool {
        if caller != NET_PD || self.waiting == Waiting::Nothing || !ws::is_reply(value) { return false; }
        let waiting = core::mem::replace(&mut self.waiting, Waiting::Nothing);
        if value == ws::REFUSED { sb.push(b"Wi-Fi: the network service refused that."); return true; }
        let mut line = [0u8; 128];
        match waiting {
            Waiting::Nothing => return false,
            Waiting::State { scan } => {
                let s = ws::State::of(value);
                let n = state_line(s, scan, &mut line);
                sb.push(&line[..n]);
                sex_pdx::serial_record!("[spindle.wifi] phase={} networks={} scans={} channels={}", s.phase, s.networks, s.scans, s.channels);
                // A scan just started has nothing finished to list yet.
                if s.networks > 0 && !(scan && s.phase == ws::SCANNING) && ask(ws::NETWORK, 0, ws::PART_INFO) {
                    self.waiting = Waiting::Network { index: 0, part: ws::PART_INFO };
                }
            }
            Waiting::Network { index, part } => {
                if value == ws::MISSING { return true; }
                match part {
                    ws::PART_INFO => { self.info = ws::Info::of(value); self.ssid = [0; 35]; }
                    ws::PART_BSSID => ws::word_bytes(value, &mut self.bssid),
                    p => ws::word_bytes(value, &mut self.ssid[((p - ws::PART_SSID) * 7) as usize..]),
                }
                // SSID parts needed for this network's length.
                let ssid_len = (self.info.ssid_len as u64).min(32);
                let last = ws::PART_SSID + (ssid_len + 6) / 7;
                let next = part + 1;
                if next < last.max(ws::PART_SSID) && ask(ws::NETWORK, index, next) {
                    self.waiting = Waiting::Network { index, part: next };
                    return true;
                }
                let n = network_line(self.info, &self.bssid, &self.ssid[..ssid_len as usize], &mut line);
                sb.push(&line[..n]);
                if index + 1 < MAX_LISTED && ask(ws::NETWORK, index + 1, ws::PART_INFO) {
                    self.waiting = Waiting::Network { index: index + 1, part: ws::PART_INFO };
                }
            }
        }
        true
    }
}
