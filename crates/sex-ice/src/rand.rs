// SPDX-License-Identifier: GPL-3.0-only
//! Transaction ids, credentials and tie-breakers from a seed the program gives (taken
//! from the system's entropy). HMAC-SHA1 over a counter: unpredictable without the seed.

use crate::stun::hmac_sha1;
use alloc::string::String;

#[derive(Clone, Debug)]
pub struct Rng { key: [u8; 32], counter: u64 }
impl Rng {
    pub fn new(seed: [u8; 32]) -> Self { Rng { key: seed, counter: 0 } }
    pub fn fill(&mut self, out: &mut [u8]) {
        for chunk in out.chunks_mut(20) {
            self.counter += 1;
            let b = hmac_sha1(&self.key, &[&self.counter.to_be_bytes()]);
            chunk.copy_from_slice(&b[..chunk.len()]);
        }
    }
    pub fn u64(&mut self) -> u64 { let mut b = [0; 8]; self.fill(&mut b); u64::from_be_bytes(b) }
    pub fn below(&mut self, n: u64) -> u64 { if n == 0 { 0 } else { self.u64() % n } }
    pub fn transaction(&mut self) -> [u8; 12] { let mut t = [0; 12]; self.fill(&mut t); t }
    /// A seed for another generator.
    pub fn fork(&mut self) -> Rng { let mut k = [0; 32]; self.fill(&mut k); Rng::new(k) }
    /// `n` ice-chars (ALPHA, DIGIT, `+`, `/`), as ice-ufrag and ice-pwd use.
    pub fn ice_chars(&mut self, n: usize) -> String {
        const SET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut b = alloc::vec![0u8; n];
        self.fill(&mut b);
        b.iter().map(|x| SET[(*x & 63) as usize] as char).collect()
    }
}
