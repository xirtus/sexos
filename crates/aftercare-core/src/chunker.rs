// SPDX-License-Identifier: GPL-3.0-only
//! FastCDC content-defined chunking (Xia et al., USENIX ATC 2016) with normalized chunking
//! (level 2). The gear table is derived from a per-repository random seed so chunk
//! boundaries do not reveal content to someone who sees only chunk sizes.

/// Chunk size bounds in bytes. `avg` is rounded down to a power of two.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChunkerParams {
    pub min: u32,
    pub avg: u32,
    pub max: u32,
}

impl Default for ChunkerParams {
    /// min 256 KiB, average 1 MiB, max 4 MiB.
    fn default() -> Self {
        ChunkerParams { min: 256 * 1024, avg: 1024 * 1024, max: 4 * 1024 * 1024 }
    }
}

impl ChunkerParams {
    pub fn is_valid(&self) -> bool {
        self.min >= 64 && self.min < self.avg && self.avg < self.max && self.max <= 64 * 1024 * 1024
    }
}

pub struct Chunker {
    gear: [u64; 256],
    min: usize,
    avg: usize,
    max: usize,
    mask_s: u64,
    mask_l: u64,
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The gear hash shifts left, so its high bits carry the most history: test those.
fn top_mask(bits: u32) -> u64 {
    if bits == 0 {
        0
    } else {
        !0u64 << (64 - bits.min(63))
    }
}

impl Chunker {
    pub fn new(p: ChunkerParams, seed: u64) -> Self {
        let mut s = seed;
        let mut gear = [0u64; 256];
        for g in gear.iter_mut() {
            *g = splitmix64(&mut s);
        }
        let bits = 31 - p.avg.leading_zeros();
        Chunker {
            gear,
            min: p.min as usize,
            avg: 1usize << bits,
            max: p.max as usize,
            mask_s: top_mask(bits + 2),
            mask_l: top_mask(bits.saturating_sub(2)),
        }
    }

    pub fn max_size(&self) -> usize {
        self.max
    }

    /// Length of the next chunk at the start of `data`. `data` must hold at least `max`
    /// bytes unless it is the end of the stream (`eof`).
    pub fn cut(&self, data: &[u8], eof: bool) -> usize {
        debug_assert!(eof || data.len() >= self.max);
        let n = data.len().min(self.max);
        if n <= self.min {
            return n;
        }
        let normal = self.avg.min(n);
        let mut h: u64 = 0;
        let mut i = self.min;
        while i < normal {
            h = (h << 1).wrapping_add(self.gear[data[i] as usize]);
            if h & self.mask_s == 0 {
                return i + 1;
            }
            i += 1;
        }
        while i < n {
            h = (h << 1).wrapping_add(self.gear[data[i] as usize]);
            if h & self.mask_l == 0 {
                return i + 1;
            }
            i += 1;
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    fn noise(n: usize, mut s: u64) -> Vec<u8> {
        (0..n).map(|_| splitmix64(&mut s) as u8).collect()
    }

    fn chunks(c: &Chunker, d: &[u8]) -> Vec<usize> {
        let mut out = Vec::new();
        let mut pos = 0;
        while pos < d.len() {
            let n = c.cut(&d[pos..], true);
            out.push(n);
            pos += n;
        }
        out
    }

    #[test]
    fn sizes_bounded_and_average_near_target() {
        let p = ChunkerParams { min: 2048, avg: 8192, max: 32768 };
        let c = Chunker::new(p, 7);
        let d = noise(4 << 20, 1);
        let v = chunks(&c, &d);
        assert!(v[..v.len() - 1].iter().all(|&n| n >= 2048 && n <= 32768));
        let avg = d.len() / v.len();
        assert!((6000..14000).contains(&avg), "avg {avg}");
    }

    #[test]
    fn boundaries_resync_after_insert() {
        let p = ChunkerParams { min: 2048, avg: 8192, max: 32768 };
        let c = Chunker::new(p, 7);
        let a = noise(1 << 20, 2);
        let mut b = a[..300_000].to_vec();
        b.extend_from_slice(b"inserted bytes");
        b.extend_from_slice(&a[300_000..]);
        let ca = chunks(&c, &a);
        let cb = chunks(&c, &b);
        // Same chunk sequence at the end: suffix boundaries survive.
        let tail = ca.iter().rev().zip(cb.iter().rev()).take_while(|(x, y)| x == y).count();
        assert!(tail * 10 > ca.len() * 6, "{tail} of {}", ca.len());
    }
}
