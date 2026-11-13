// SPDX-License-Identifier: GPL-3.0-only
//! Encryption for a SexOS volume at rest: XTS-AES-256 per block (IEEE 1619, the mode disk
//! encryption uses everywhere), with the volume's own random 512-bit key kept in a header
//! wrapped by a key derived from the password with Argon2id (RFC 9106).
//!
//! The password never touches the data: changing it rewraps 64 bytes in the header. A wrong
//! password is told apart from damage by a check value of the unwrapped key, so it can be
//! refused before a single block is read with the wrong key.
#![cfg_attr(not(test), no_std)]
extern crate alloc;

use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
use aes::{Aes256, Block};
use sha2::{Digest, Sha256};

pub const MAGIC: &[u8; 8] = b"SEXCRYPT";
pub const VERSION: u32 = 1;
/// Bytes of header at the start of an encrypted volume; data follows.
pub const HEADER: usize = 4096;
pub const KEY: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// No SEXCRYPT header: not an encrypted volume.
    NotEncrypted,
    WrongPassword,
    /// A header we can't read (another version, impossible parameters).
    Corrupt,
}

/// Argon2id cost: memory in KiB, passes, lanes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cost { pub memory_kib: u32, pub passes: u32, pub lanes: u32 }
impl Cost {
    /// About a second on one core of a laptop; more under QEMU's software CPU.
    pub const DEFAULT: Cost = Cost { memory_kib: 64 * 1024, passes: 3, lanes: 1 };
}

/// XTS-AES-256 with a 512-bit key: the first half encrypts, the second makes tweaks.
pub struct Xts { data: Aes256, tweak: Aes256 }

impl Xts {
    pub fn new(key: &[u8; KEY]) -> Xts {
        Xts { data: Aes256::new_from_slice(&key[..32]).unwrap(), tweak: Aes256::new_from_slice(&key[32..]).unwrap() }
    }
    fn first_tweak(&self, unit: u64) -> [u8; 16] {
        let mut t = [0u8; 16];
        t[..8].copy_from_slice(&unit.to_le_bytes());
        let mut b = Block::from(t);
        self.tweak.encrypt_block(&mut b);
        b.into()
    }
    /// Encrypts one data unit (a block) in place; `data.len()` is a multiple of 16.
    pub fn encrypt(&self, unit: u64, data: &mut [u8]) { self.run(unit, data, true) }
    pub fn decrypt(&self, unit: u64, data: &mut [u8]) { self.run(unit, data, false) }
    fn run(&self, unit: u64, data: &mut [u8], enc: bool) {
        assert!(data.len() % 16 == 0 && !data.is_empty());
        let mut t = self.first_tweak(unit);
        for chunk in data.chunks_exact_mut(16) {
            for (c, k) in chunk.iter_mut().zip(&t) { *c ^= k; }
            let mut b = Block::clone_from_slice(chunk);
            if enc { self.data.encrypt_block(&mut b) } else { self.data.decrypt_block(&mut b) }
            for ((c, x), k) in chunk.iter_mut().zip(b.iter()).zip(&t) { *c = x ^ k; }
            // Multiply the tweak by x in GF(2^128), little-endian.
            let carry = t[15] >> 7;
            for i in (1..16).rev() { t[i] = (t[i] << 1) | (t[i - 1] >> 7); }
            t[0] = (t[0] << 1) ^ (carry * 0x87);
        }
    }
}

fn kek(password: &[u8], salt: &[u8; 32], cost: Cost) -> Result<[u8; KEY], Error> {
    let params = argon2::Params::new(cost.memory_kib, cost.passes, cost.lanes, Some(KEY)).map_err(|_| Error::Corrupt)?;
    let a = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut out = [0u8; KEY];
    a.hash_password_into(password, salt, &mut out).map_err(|_| Error::Corrupt)?;
    Ok(out)
}
fn check(master: &[u8; KEY]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"sexcrypt key check v1");
    h.update(master);
    h.finalize().into()
}
/// The unit number the wrapped key is encrypted under: never a data block's.
const WRAP_UNIT: u64 = u64::MAX;

/// A new header for `master`, wrapped by `password`. `salt` and `master` must be random.
pub fn header(password: &[u8], salt: &[u8; 32], master: &[u8; KEY], cost: Cost) -> Result<[u8; HEADER], Error> {
    let mut h = [0u8; HEADER];
    h[..8].copy_from_slice(MAGIC);
    h[8..12].copy_from_slice(&VERSION.to_le_bytes());
    h[12..16].copy_from_slice(&cost.memory_kib.to_le_bytes());
    h[16..20].copy_from_slice(&cost.passes.to_le_bytes());
    h[20..24].copy_from_slice(&cost.lanes.to_le_bytes());
    h[32..64].copy_from_slice(salt);
    let mut wrapped = *master;
    Xts::new(&kek(password, salt, cost)?).encrypt(WRAP_UNIT, &mut wrapped);
    h[64..128].copy_from_slice(&wrapped);
    h[128..160].copy_from_slice(&check(master));
    Ok(h)
}
/// The cost a header asks for, or why it can't be read.
pub fn cost(h: &[u8]) -> Result<Cost, Error> {
    if h.len() < HEADER || &h[..8] != MAGIC { return Err(Error::NotEncrypted); }
    let word = |at: usize| u32::from_le_bytes(h[at..at + 4].try_into().unwrap());
    if word(8) != VERSION { return Err(Error::Corrupt); }
    let c = Cost { memory_kib: word(12), passes: word(16), lanes: word(20) };
    // Bounds that keep a damaged header from asking for a gigabyte or an hour.
    if !(8..=1024 * 1024).contains(&c.memory_kib) || !(1..=16).contains(&c.passes) || !(1..=8).contains(&c.lanes) { return Err(Error::Corrupt); }
    Ok(c)
}
/// The volume's key, if `password` is the one it was wrapped with.
pub fn unlock(h: &[u8], password: &[u8]) -> Result<[u8; KEY], Error> {
    let cost = cost(h)?;
    let salt: [u8; 32] = h[32..64].try_into().unwrap();
    let mut master: [u8; KEY] = h[64..128].try_into().unwrap();
    Xts::new(&kek(password, &salt, cost)?).decrypt(WRAP_UNIT, &mut master);
    if check(&master)[..] != h[128..160] { return Err(Error::WrongPassword); }
    Ok(master)
}
/// The same volume key under a new password (and a new salt).
pub fn rewrap(h: &[u8], old: &[u8], new: &[u8], salt: &[u8; 32]) -> Result<[u8; HEADER], Error> {
    let master = unlock(h, old)?;
    header(new, salt, &master, cost(h)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sha(b: &[u8]) -> String { Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect() }

    #[test]
    fn xts_matches_an_independent_implementation() {
        // Reference values from OpenSSL (Python `cryptography`, XTS mode, 4096-byte units).
        let key: [u8; 64] = core::array::from_fn(|i| i as u8);
        let plain: Vec<u8> = (0..4096).map(|i| ((i * 7 + 3) & 255) as u8).collect();
        let x = Xts::new(&key);
        for (unit, want) in [(0u64, "96b1135aa7d85b8be88ae0c31c00230fd19de4b815f406a7e55a27a4582454f4"),
                             (1, "6af740bec4675d66aa3ee6b3029fd49c62f5c669aba292f70c11aad90ae03797"),
                             (0x1_2345_6789, "1b9958015f862f3b8e0415dd3ef35a25453053adebcddf544dd00369e953ff51")] {
            let mut d = plain.clone();
            x.encrypt(unit, &mut d);
            assert_eq!(sha(&d), want, "unit {unit}");
            x.decrypt(unit, &mut d);
            assert_eq!(d, plain);
        }
    }
    #[test]
    fn argon2id_matches_an_independent_implementation() {
        let salt16: &[u8] = b"sexcrypt-salt-16";
        let params = argon2::Params::new(256, 2, 1, Some(64)).unwrap();
        let mut out = [0u8; 64];
        argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params).hash_password_into(b"correct horse", salt16, &mut out).unwrap();
        let hex: String = out.iter().map(|x| format!("{x:02x}")).collect();
        assert_eq!(hex, "8b6144174e03a72dd5885a1de3c4cf8693ebb4d1825228c3a7244877a1b2e02a217b8ceb65a064f97dd757ce3e8890ac56d298cc9a4ff72bd730987fbe261bb7");
    }
    #[test]
    fn right_password_unlocks_wrong_one_does_not() {
        let cost = Cost { memory_kib: 64, passes: 1, lanes: 1 };
        let master: [u8; KEY] = core::array::from_fn(|i| (i * 13 + 5) as u8);
        let h = header(b"hunter2 but longer", &[9; 32], &master, cost).unwrap();
        assert_eq!(unlock(&h, b"hunter2 but longer").unwrap(), master);
        assert_eq!(unlock(&h, b"hunter3 but longer").unwrap_err(), Error::WrongPassword);
        assert_eq!(unlock(&[0u8; HEADER], b"x").unwrap_err(), Error::NotEncrypted);
        let h2 = rewrap(&h, b"hunter2 but longer", b"new one", &[7; 32]).unwrap();
        assert_eq!(unlock(&h2, b"new one").unwrap(), master);
        assert_eq!(unlock(&h2, b"hunter2 but longer").unwrap_err(), Error::WrongPassword);
        // The key itself is nowhere in the header.
        assert!(!h.windows(16).any(|w| w == &master[..16]));
        let mut bad = h;
        bad[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(unlock(&bad, b"hunter2 but longer").unwrap_err(), Error::Corrupt);
    }
}
