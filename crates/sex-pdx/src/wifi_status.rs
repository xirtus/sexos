// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Wi-Fi as sexnet sees it, for the desktop: Spindle's `wifi` command (and later the
//! status bar) ask sexnet what its Intel Wi-Fi chip is doing and which networks its
//! last scan heard, and may ask for a fresh scan.
//!
//! Only the desktop asks: Shell, SilkBar and Spindle rows. A network list says where
//! the machine is, so programs get nothing. Connecting is a later stage (W5).
//!
//! rdi = op `OP`, rsi = action, rdx = a, r10 = b. sexnet replies with one word, whose
//! top byte is `TAG` (an answer) or `NO` (a refusal, or no such network), so a caller
//! can tell it from sexnet's other replies.

/// Shell, SilkBar or Spindle to sexnet.
pub const OP: u64 = 0x2E8;

/// Replies with the state word.
pub const STATE: u64 = 1;
/// Starts a fresh scan when the chip is ready and not already scanning. Replies with
/// the state word (phase `SCANNING` when it started).
pub const SCAN: u64 = 2;
/// Network `a` of the last (or current) scan, part `b`: `PART_INFO`, `PART_BSSID`,
/// or `PART_SSID + k` for SSID bytes 7k..7k+7. Replies `MISSING` past the end.
pub const NETWORK: u64 = 3;
/// Joins network `a` of the last finished scan: onto its channel (W5a), then
/// authentication and association (W5b). Shell and Spindle only. Replies with the state word
/// (phase `JOINING` when it started).
pub const JOIN: u64 = 4;
/// Part of the passphrase for the next `JOIN` (WPA2): `a` = part k (bits 0..8) and
/// the whole length (8..16), `b` = bytes 7k..7k+7 as `bytes_word`. Shell and
/// Spindle only. sexnet keeps it until the join, then only the key derived from it.
/// A `JOIN` of a WPA2 network without one is answered `NEEDS_PASSWORD`.
pub const PASSWORD: u64 = 5;
/// 9 × 7 bytes hold WPA2's 63-character maximum.
pub const PASSWORD_PARTS: u64 = 9;
pub const PASSWORD_MAX: usize = 63;
pub const PASSWORD_MIN: usize = 8;

pub const PART_INFO: u64 = 0;
pub const PART_BSSID: u64 = 1;
pub const PART_SSID: u64 = 2;
/// SSID parts: 5 × 7 bytes hold the 32-byte maximum.
pub const SSID_PARTS: u64 = 5;

pub const TAG: u64 = 0x57 << 56;
pub const NO: u64 = 0x58 << 56;
pub const REFUSED: u64 = NO | 1;
pub const MISSING: u64 = NO | 2;
pub const NEEDS_PASSWORD: u64 = NO | 3;
pub const fn is_reply(word: u64) -> bool { word >> 56 == TAG >> 56 || word >> 56 == NO >> 56 }

/// What the chip is doing.
pub const NONE: u8 = 0; // no Intel Wi-Fi chip (or its firmware is missing)
pub const STARTING: u8 = 1; // firmware loading and init commands
pub const SCANNING: u8 = 2;
pub const READY: u8 = 3; // the last scan finished
pub const FAILED: u8 = 4; // start-up, the last scan or the last join failed
pub const JOINING: u8 = 5;
pub const JOINED: u8 = 6; // on the network's channel, not yet authenticated
pub const ASSOCIATING: u8 = 7; // authentication and association frames
pub const ASSOCIATED: u8 = 8; // associated with an open network (data path: W5e)
pub const HANDSHAKE: u8 = 9; // WPA2 4-way handshake and key install
pub const CONNECTED: u8 = 10; // keys installed (data path: W5e)

/// The state word: phase (bits 0..8), networks heard (8..16), scans started (16..32),
/// channels in a scan (32..40).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct State { pub phase: u8, pub networks: u8, pub scans: u16, pub channels: u8 }
impl State {
    pub const fn word(self) -> u64 {
        TAG | self.phase as u64 | (self.networks as u64) << 8 | (self.scans as u64) << 16 | (self.channels as u64) << 32
    }
    pub const fn of(word: u64) -> State {
        State { phase: word as u8, networks: (word >> 8) as u8, scans: (word >> 16) as u16, channels: (word >> 32) as u8 }
    }
}

/// Security, as `PART_INFO` carries it.
pub const OPEN: u8 = 0;
pub const WPA2: u8 = 1;
pub const WPA3: u8 = 2;
pub const WPA2_WPA3: u8 = 3;
pub const UNSUPPORTED: u8 = 4;

pub const fn security_name(s: u8) -> &'static str {
    match s { OPEN => "open", WPA2 => "WPA2", WPA3 => "WPA3", WPA2_WPA3 => "WPA2/WPA3", _ => "unsupported" }
}

/// `PART_INFO`: channel (bits 0..8), signal in dBm as i8 (8..16), security (16..24),
/// SSID length (24..32), beacons heard (32..48).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Info { pub channel: u8, pub rssi: i8, pub security: u8, pub ssid_len: u8, pub beacons: u16 }
impl Info {
    pub const fn word(self) -> u64 {
        TAG | self.channel as u64 | (self.rssi as u8 as u64) << 8 | (self.security as u64) << 16
            | (self.ssid_len as u64) << 24 | (self.beacons as u64) << 32
    }
    pub const fn of(word: u64) -> Info {
        Info { channel: word as u8, rssi: (word >> 8) as u8 as i8, security: (word >> 16) as u8,
            ssid_len: (word >> 24) as u8, beacons: (word >> 32) as u16 }
    }
}

/// Up to 7 bytes in bits 0..56.
pub fn bytes_word(bytes: &[u8]) -> u64 {
    let mut w = 0u64;
    for (i, &b) in bytes.iter().take(7).enumerate() { w |= (b as u64) << (8 * i); }
    TAG | w
}
pub fn word_bytes(word: u64, out: &mut [u8]) {
    for (i, b) in out.iter_mut().take(7).enumerate() { *b = (word >> (8 * i)) as u8; }
}

/// The answer for `NETWORK` part `part` of a network with these facts.
pub fn network_part(info: Info, bssid: &[u8; 6], ssid: &[u8], part: u64) -> u64 {
    match part {
        PART_INFO => info.word(),
        PART_BSSID => bytes_word(bssid),
        p if p >= PART_SSID && p < PART_SSID + SSID_PARTS => {
            let from = ((p - PART_SSID) * 7) as usize;
            bytes_word(ssid.get(from..).unwrap_or(&[]))
        }
        _ => MISSING,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_round_trip() {
        let s = State { phase: READY, networks: 12, scans: 3, channels: 67 };
        assert_eq!(State::of(s.word()), s);
        assert!(is_reply(s.word()) && is_reply(REFUSED) && is_reply(MISSING) && is_reply(NEEDS_PASSWORD));
        // 9 parts carry the longest passphrase.
        assert!(PASSWORD_PARTS as usize * 7 >= PASSWORD_MAX);
        let i = Info { channel: 149, rssi: -71, security: WPA2_WPA3, ssid_len: 32, beacons: 40 };
        assert_eq!(Info::of(i.word()), i);
        // Other sexnet replies (network jobs, DNS, web) are not Wi-Fi replies.
        for tag in [0x78u64, 0x79, 0xE8, 0xE9, 0xF6, 0x4D] { assert!(!is_reply(tag << 56)); }
    }

    #[test]
    fn ssid_parts_carry_32_bytes() {
        let ssid: [u8; 32] = core::array::from_fn(|i| b'a' + i as u8 % 26);
        let info = Info { channel: 6, rssi: -40, security: WPA3, ssid_len: 32, beacons: 1 };
        let mut back = [0u8; 35];
        for k in 0..SSID_PARTS {
            word_bytes(network_part(info, &[0; 6], &ssid, PART_SSID + k), &mut back[k as usize * 7..]);
        }
        assert_eq!(&back[..32], &ssid);
        let mut mac = [0u8; 7];
        word_bytes(network_part(info, &[1, 2, 3, 4, 5, 0xfe], &ssid, PART_BSSID), &mut mac);
        assert_eq!(&mac[..6], &[1, 2, 3, 4, 5, 0xfe]);
        assert_eq!(network_part(info, &[0; 6], &ssid, PART_SSID + SSID_PARTS), MISSING);
        // A short SSID leaves the later parts empty.
        assert_eq!(network_part(info, &[0; 6], b"home", PART_SSID + 1), TAG);
    }
}
