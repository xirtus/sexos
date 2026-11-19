// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Receive-side scaling: the NIC hashes each packet's addresses and ports with the
//! Toeplitz function, looks the low bits up in an indirection table and puts the
//! packet on that RX queue. Every packet of one flow lands on one queue, so one core
//! owns each connection's state and no lock is shared between cores. SexNet computes
//! the same hash in software to pick the TX queue and lane for an outgoing flow, and
//! for NICs without RSS.
//!
//! Also here: the Intel gigabit register layout for several queues (82576/I350
//! "igb" and I225/I226 "igc" share it).

/// The key Microsoft's RSS specification uses for its verification suite; Intel and
/// most drivers default to it. 40 bytes covers IPv6 addresses and ports.
pub const DEFAULT_KEY: [u8; 40] = [
    0x6d, 0x5a, 0x56, 0xda, 0x25, 0x5b, 0x0e, 0xc2, 0x41, 0x67, 0x25, 0x3d, 0x43, 0xa3, 0x8f, 0xb0,
    0xd0, 0xca, 0x2b, 0xcb, 0xae, 0x7b, 0x30, 0xb4, 0x77, 0xcb, 0x2d, 0xa3, 0x80, 0x30, 0xf2, 0x0c,
    0x6a, 0x42, 0xb7, 0x3b, 0xbe, 0xac, 0x01, 0xfa,
];

/// Toeplitz hash of `input` under `key` (input at most `key.len() - 4` bytes).
pub fn toeplitz(key: &[u8], input: &[u8]) -> u32 {
    debug_assert!(input.len() + 4 <= key.len());
    let mut result = 0u32;
    // The 32-bit window of the key that lines up with the current input bit.
    let mut window = u32::from_be_bytes([key[0], key[1], key[2], key[3]]);
    for (i, byte) in input.iter().enumerate() {
        let next = key.get(i + 4).copied().unwrap_or(0);
        for bit in 0..8 {
            if byte & (0x80 >> bit) != 0 { result ^= window; }
            window = window << 1 | ((next >> (7 - bit)) & 1) as u32;
        }
    }
    result
}

/// Hash of an IPv4 TCP/UDP flow, as the NIC computes it: source address, destination
/// address, source port, destination port, all in network order.
pub fn ipv4_flow(key: &[u8], src: [u8; 4], dst: [u8; 4], sport: u16, dport: u16) -> u32 {
    let mut input = [0u8; 12];
    input[..4].copy_from_slice(&src);
    input[4..8].copy_from_slice(&dst);
    input[8..10].copy_from_slice(&sport.to_be_bytes());
    input[10..12].copy_from_slice(&dport.to_be_bytes());
    toeplitz(key, &input)
}

/// Hash of the IPv4 addresses only (fragments and protocols without ports).
pub fn ipv4_addresses(key: &[u8], src: [u8; 4], dst: [u8; 4]) -> u32 {
    let mut input = [0u8; 8];
    input[..4].copy_from_slice(&src);
    input[4..].copy_from_slice(&dst);
    toeplitz(key, &input)
}

/// Hash of an IPv6 TCP/UDP flow.
pub fn ipv6_flow(key: &[u8], src: [u8; 16], dst: [u8; 16], sport: u16, dport: u16) -> u32 {
    let mut input = [0u8; 36];
    input[..16].copy_from_slice(&src);
    input[16..32].copy_from_slice(&dst);
    input[32..34].copy_from_slice(&sport.to_be_bytes());
    input[34..36].copy_from_slice(&dport.to_be_bytes());
    toeplitz(key, &input)
}

/// Indirection table entries on Intel gigabit NICs.
pub const RETA_ENTRIES: usize = 128;

/// Spreads hash buckets over `queues` queues round robin.
pub fn spread(queues: u8) -> [u8; RETA_ENTRIES] {
    let q = queues.max(1);
    core::array::from_fn(|i| (i % q as usize) as u8)
}

/// The queue a hash goes to.
pub fn queue_of(hash: u32, table: &[u8; RETA_ENTRIES]) -> u8 { table[hash as usize % RETA_ENTRIES] }

/// Intel igb/igc registers for queue `n` and for RSS.
pub mod intel {
    pub const fn rdbal(n: u32) -> u32 { 0xC000 + 0x40 * n }
    pub const fn rdbah(n: u32) -> u32 { 0xC004 + 0x40 * n }
    pub const fn rdlen(n: u32) -> u32 { 0xC008 + 0x40 * n }
    pub const fn srrctl(n: u32) -> u32 { 0xC00C + 0x40 * n }
    pub const fn rdh(n: u32) -> u32 { 0xC010 + 0x40 * n }
    pub const fn rdt(n: u32) -> u32 { 0xC018 + 0x40 * n }
    pub const fn rxdctl(n: u32) -> u32 { 0xC028 + 0x40 * n }
    pub const fn tdbal(n: u32) -> u32 { 0xE000 + 0x40 * n }
    pub const fn tdbah(n: u32) -> u32 { 0xE004 + 0x40 * n }
    pub const fn tdlen(n: u32) -> u32 { 0xE008 + 0x40 * n }
    pub const fn tdh(n: u32) -> u32 { 0xE010 + 0x40 * n }
    pub const fn tdt(n: u32) -> u32 { 0xE018 + 0x40 * n }
    pub const fn txdctl(n: u32) -> u32 { 0xE028 + 0x40 * n }
    /// Multiple receive queues command.
    pub const MRQC: u32 = 0x5818;
    /// Redirection table: 32 registers, 4 one-byte entries each.
    pub const RETA: u32 = 0x5C00;
    /// RSS random key: 10 registers.
    pub const RSSRK: u32 = 0x5C80;
    /// MRQC: RSS on, hashing IPv4, IPv4/TCP, IPv6, IPv6/TCP, IPv4/UDP, IPv6/UDP.
    pub const MRQC_RSS: u32 = 0x2 | 1 << 16 | 1 << 17 | 1 << 20 | 1 << 21 | 1 << 22 | 1 << 23;

    /// The ten RSSRK words for `key` (little-endian per register, as the NIC reads it).
    pub fn key_words(key: &[u8; 40]) -> [u32; 10] {
        core::array::from_fn(|i| u32::from_le_bytes([key[4 * i], key[4 * i + 1], key[4 * i + 2], key[4 * i + 3]]))
    }

    /// The 32 RETA words for a table of queue numbers.
    pub fn reta_words(table: &[u8; super::RETA_ENTRIES]) -> [u32; 32] {
        core::array::from_fn(|i| u32::from_le_bytes([table[4 * i], table[4 * i + 1], table[4 * i + 2], table[4 * i + 3]]))
    }
}
