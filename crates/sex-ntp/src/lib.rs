// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! SNTPv4 client codec (RFC 4330, on-wire math from RFC 5905). No I/O, no clock: the
//! caller sends [`request`]'s 48 bytes over UDP to port [`PORT`], keeps the [`Query`],
//! and hands the reply to [`Query::answer`], with its own clock readings in Unix
//! milliseconds. What comes back is how far the local clock is off, and the round trip.
//!
//! A reply is refused unless it answers this query (its origin timestamp is our
//! transmit timestamp, which carries random low bits), is a server reply (mode 4) of
//! version 3 or 4, from a synchronized server (stratum 1..=15, leap indicator not 3),
//! and its timestamps make sense. A kiss-o'-death (stratum 0) is told apart, so the
//! caller can back off.
#![no_std]

pub const PORT: u16 = 123;
pub const PACKET: usize = 48;
/// Seconds from the NTP era 0 epoch (1900) to the Unix epoch.
pub const UNIX_EPOCH: u64 = 2_208_988_800;
/// Replies taking longer than this round trip are not trusted.
pub const MAX_DELAY_MS: i64 = 5_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// Not 48 bytes or more.
    Short,
    /// Not mode 4, or a version other than 3 or 4.
    NotReply,
    /// The origin timestamp is not this query's: a stray or spoofed reply.
    NotOurs,
    /// Kiss-o'-death: the server asks us to go away (its four-letter code).
    Kiss([u8; 4]),
    /// The server is not synchronized (leap indicator 3, or stratum 16 and up).
    Unsynchronized,
    /// Zero, out of order, or impossible timestamps; or a round trip too long.
    Times,
}

/// What a checked reply says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    /// Add this to the local clock to get the server's time (ms).
    pub offset_ms: i64,
    /// Round trip, less the server's own time (ms).
    pub delay_ms: i64,
    pub stratum: u8,
    /// The server's time when it sent the reply, in Unix ms.
    pub server_ms: u64,
}

/// A query in flight: what to expect back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Query { transmit: u64 }

/// Unix milliseconds to a 64-bit NTP timestamp (era 0, wraps in 2036 as NTP does).
pub fn timestamp(unix_ms: u64) -> u64 {
    let seconds = (unix_ms / 1000).wrapping_add(UNIX_EPOCH) & 0xFFFF_FFFF;
    let fraction = ((unix_ms % 1000) << 32) / 1000;
    seconds << 32 | fraction
}
/// A 64-bit NTP timestamp to Unix milliseconds, taking the era nearest `near_ms`
/// (so 2036's wrap reads right). Zero is "no time".
pub fn unix_ms(stamp: u64, near_ms: u64) -> Option<u64> {
    if stamp == 0 { return None; }
    let seconds = stamp >> 32;
    let millis = ((stamp & 0xFFFF_FFFF) * 1000 + (1 << 31)) >> 32; // nearest ms
    let near = (near_ms / 1000 + UNIX_EPOCH) as i64;
    let era = 1i64 << 32;
    let mut s = (near & !(era - 1)) + seconds as i64;
    if s - near > era / 2 { s -= era; } else if near - s > era / 2 { s += era; }
    let unix = s - UNIX_EPOCH as i64;
    if unix < 0 { return None; }
    Some(unix as u64 * 1000 + millis)
}

/// The 48-byte client request, sent at local Unix time `now_ms`. `nonce` fills the
/// sub-millisecond bits of the transmit timestamp, so a reply must echo something an
/// off-path sender cannot guess; give it fresh random bits each time.
pub fn request(now_ms: u64, nonce: u32) -> ([u8; PACKET], Query) {
    let mut p = [0u8; PACKET];
    p[0] = 4 << 3 | 3; // LI 0, version 4, mode 3 (client)
    // Below a millisecond of the fraction: about 2^22 units, room for 22 random bits.
    let transmit = timestamp(now_ms) ^ (nonce as u64 & 0x3F_FFFF);
    p[40..48].copy_from_slice(&transmit.to_be_bytes());
    (p, Query { transmit })
}

impl Query {
    /// Checks `reply`, received at local Unix time `received_ms`; `sent_ms` is the local
    /// time the request went out (as given to [`request`]).
    pub fn answer(&self, reply: &[u8], sent_ms: u64, received_ms: u64) -> Result<Sample, Refused> {
        if reply.len() < PACKET { return Err(Refused::Short); }
        let (leap, version, mode) = (reply[0] >> 6, reply[0] >> 3 & 7, reply[0] & 7);
        if mode != 4 || !(3..=4).contains(&version) { return Err(Refused::NotReply); }
        let word = |at: usize| u64::from_be_bytes(reply[at..at + 8].try_into().unwrap());
        if word(24) != self.transmit { return Err(Refused::NotOurs); }
        let stratum = reply[1];
        if stratum == 0 { return Err(Refused::Kiss(reply[12..16].try_into().unwrap())); }
        if leap == 3 || stratum >= 16 { return Err(Refused::Unsynchronized); }
        if received_ms < sent_ms { return Err(Refused::Times); }
        let (Some(t2), Some(t3)) = (unix_ms(word(32), sent_ms), unix_ms(word(40), sent_ms)) else { return Err(Refused::Times) };
        if t3 < t2 { return Err(Refused::Times); }
        let (t1, t4) = (sent_ms as i64, received_ms as i64);
        let (t2, t3) = (t2 as i64, t3 as i64);
        let delay = (t4 - t1) - (t3 - t2);
        if !(0..=MAX_DELAY_MS).contains(&delay) { return Err(Refused::Times); }
        let offset = ((t2 - t1) + (t3 - t4)) / 2;
        Ok(Sample { offset_ms: offset, delay_ms: delay, stratum, server_ms: t3 as u64 })
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    const NOW: u64 = 1_763_000_000_123; // 2025-11-13

    fn reply(q: &[u8; PACKET], stratum: u8, rx_ms: u64, tx_ms: u64) -> [u8; PACKET] {
        let mut r = [0u8; PACKET];
        r[0] = 4 << 3 | 4;
        r[1] = stratum;
        r[24..32].copy_from_slice(&q[40..48]);
        r[32..40].copy_from_slice(&timestamp(rx_ms).to_be_bytes());
        r[40..48].copy_from_slice(&timestamp(tx_ms).to_be_bytes());
        r
    }

    #[test]
    fn timestamps_round_trip_and_cross_the_2036_era() {
        for ms in [0, 1, 999, NOW, 2_085_978_495_999, 2_085_978_496_001, 2_085_978_497_250, 4_102_444_799_000] {
            assert_eq!(unix_ms(timestamp(ms), ms), Some(ms), "{ms}");
        }
        // 2036-02-07T06:28:16Z is NTP second 0 of era 1 (and its exact first instant is
        // the all-zero "no time" stamp, as in NTP itself).
        assert_eq!(timestamp(2_085_978_496_000) >> 32, 0);
        assert_eq!(unix_ms(timestamp(2_085_978_496_500), 2_085_978_400_000), Some(2_085_978_496_500));
        assert_eq!(unix_ms(0, NOW), None);
    }

    #[test]
    fn a_reply_gives_offset_and_delay() {
        let (q, query) = request(NOW, 0x1234);
        assert_eq!(q[0], 0x23);
        // The server is 90 s ahead; 40 ms each way, 2 ms inside the server.
        let ahead = 90_000;
        let r = reply(&q, 2, NOW + 40 + ahead, NOW + 42 + ahead);
        let s = query.answer(&r, NOW, NOW + 82).unwrap();
        assert_eq!((s.offset_ms, s.delay_ms, s.stratum), (ahead as i64, 80, 2));
        // A slow local clock: the server is behind.
        let r = reply(&q, 1, NOW - 3_600_000, NOW - 3_600_000);
        assert_eq!(query.answer(&r, NOW, NOW + 10).unwrap().offset_ms, -3_600_000 - 5);
    }

    #[test]
    fn stray_spoofed_and_unsynchronized_replies_are_refused() {
        let (q, query) = request(NOW, 77);
        let good = reply(&q, 2, NOW + 1, NOW + 1);
        assert!(query.answer(&good, NOW, NOW + 2).is_ok());
        assert_eq!(query.answer(&good[..47], NOW, NOW + 2), Err(Refused::Short));
        let mut r = good; r[0] = 4 << 3 | 3; assert_eq!(query.answer(&r, NOW, NOW + 2), Err(Refused::NotReply));
        let mut r = good; r[0] = 2 << 3 | 4; assert_eq!(query.answer(&r, NOW, NOW + 2), Err(Refused::NotReply));
        let mut r = good; r[31] ^= 1; assert_eq!(query.answer(&r, NOW, NOW + 2), Err(Refused::NotOurs));
        let (_, other) = request(NOW, 78); assert_eq!(other.answer(&good, NOW, NOW + 2), Err(Refused::NotOurs));
        let mut r = good; r[1] = 0; r[12..16].copy_from_slice(b"RATE"); assert_eq!(query.answer(&r, NOW, NOW + 2), Err(Refused::Kiss(*b"RATE")));
        let mut r = good; r[1] = 16; assert_eq!(query.answer(&r, NOW, NOW + 2), Err(Refused::Unsynchronized));
        let mut r = good; r[0] |= 3 << 6; assert_eq!(query.answer(&r, NOW, NOW + 2), Err(Refused::Unsynchronized));
        let mut r = good; r[40..48].fill(0); assert_eq!(query.answer(&r, NOW, NOW + 2), Err(Refused::Times));
        // Sent after it was received, server answered before it heard, or too slow.
        assert_eq!(query.answer(&reply(&q, 2, NOW + 5, NOW + 4), NOW, NOW + 9), Err(Refused::Times));
        assert_eq!(query.answer(&good, NOW, NOW - 1), Err(Refused::Times));
        assert_eq!(query.answer(&good, NOW, NOW + 6_000), Err(Refused::Times));
    }
}
