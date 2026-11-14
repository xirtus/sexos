// SPDX-License-Identifier: GPL-3.0-only
//! Random numbers for keys, nonces and salts: the CPU's hardware generator (RDRAND),
//! mixed through SHA-256 with a running pool and a counter, as apps/ssh does. A
//! computer without RDRAND has nothing safe to make keys from, so the app refuses to
//! create or change a repository there.
use rand_core::{CryptoRng, RngCore};
use sha2::{Digest, Sha256};

pub struct HwRng {
    pool: [u8; 32],
    n: u64,
}

impl HwRng {
    pub fn new() -> Option<HwRng> {
        if !has_rdrand() {
            return None;
        }
        let mut pool = [0u8; 32];
        rdrand(&mut pool);
        Some(HwRng { pool, n: 0 })
    }
}

#[cfg(target_arch = "x86_64")]
fn has_rdrand() -> bool {
    core::arch::x86_64::__cpuid(1).ecx & (1 << 30) != 0
}
#[cfg(not(target_arch = "x86_64"))]
fn has_rdrand() -> bool {
    false
}

#[cfg(target_arch = "x86_64")]
fn rdrand(out: &mut [u8]) {
    for chunk in out.chunks_mut(8) {
        let mut v = 0u64;
        // SAFETY: RDRAND exists (checked in `new`); retried as Intel advises.
        for _ in 0..10 {
            if unsafe { core::arch::x86_64::_rdrand64_step(&mut v) } == 1 {
                break;
            }
        }
        chunk.copy_from_slice(&v.to_le_bytes()[..chunk.len()]);
    }
}
#[cfg(not(target_arch = "x86_64"))]
fn rdrand(_out: &mut [u8]) {}

impl RngCore for HwRng {
    fn next_u32(&mut self) -> u32 {
        let mut b = [0u8; 4];
        self.fill_bytes(&mut b);
        u32::from_le_bytes(b)
    }
    fn next_u64(&mut self) -> u64 {
        let mut b = [0u8; 8];
        self.fill_bytes(&mut b);
        u64::from_le_bytes(b)
    }
    fn fill_bytes(&mut self, out: &mut [u8]) {
        for chunk in out.chunks_mut(32) {
            let mut fresh = [0u8; 32];
            rdrand(&mut fresh);
            self.n += 1;
            let block: [u8; 32] = Sha256::new().chain_update(self.pool).chain_update(fresh).chain_update(self.n.to_le_bytes()).finalize().into();
            chunk.copy_from_slice(&block[..chunk.len()]);
            self.pool = Sha256::digest(block).into();
        }
    }
    fn try_fill_bytes(&mut self, out: &mut [u8]) -> Result<(), rand_core::Error> {
        self.fill_bytes(out);
        Ok(())
    }
}

impl CryptoRng for HwRng {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinct_blocks() {
        let Some(mut r) = HwRng::new() else { return };
        let (mut a, mut b) = ([0u8; 40], [0u8; 40]);
        r.fill_bytes(&mut a);
        r.fill_bytes(&mut b);
        assert_ne!(a, b);
        assert_ne!(a[..32], a[8..40]);
    }
}
